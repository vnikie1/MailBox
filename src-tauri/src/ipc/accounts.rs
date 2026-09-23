//! The account command surface. docs/03 §4, docs/04 Phase 4.
//!
//! The seam holds here as strictly as anywhere: **no command in this file takes or returns a
//! secret.** The UI sends an email address, a provider and — for a password account — a
//! password *in*, which then goes straight to the Credential Manager and is dropped. Nothing
//! comes back out. There is deliberately no `credential_get`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use ts_rs::TS;

use crate::accounts::{
    self,
    autodiscover::{self, Discovered, DiscoverySource},
    credentials::{self, Kind, Secret},
    oauth,
    provider::{self, AuthKind, Provider, ProviderInfo, Security, ServerSettings},
    store::{self, AccountDetail, NewAccount},
    verify::{self, Attempt, DiagnosticReport},
    ClientSource,
};
use crate::db::Db;

use super::mail::AppError;

type Response<T> = Result<T, AppError>;

fn bad_request(message: &str) -> AppError {
    AppError {
        code: "badRequest".into(),
        message: message.into(),
    }
}

fn resolve(id: &str) -> Result<Provider, AppError> {
    Provider::from_id(id).ok_or_else(|| bad_request("That provider is not one Halcyon supports."))
}

/// Turns an OAuth failure into something the UI can both branch on and show.
///
/// `needsReauth` is the code docs/03 §7 asks for: the UI raises a re-authenticate banner on
/// it rather than a generic error toast, because the two need different buttons.
impl From<oauth::OAuthError> for AppError {
    fn from(error: oauth::OAuthError) -> Self {
        let needs_reauth = oauth::requires_reauthentication(&error);

        // Logged as the error's own Display, which by construction never contains a token —
        // `Secret` has no Display, and the variants carry provider ids and error codes.
        tracing::warn!(%error, "oauth failed");

        let (code, message) = match &error {
            // Before the re-auth arm, and that ordering is the whole point. `invalid_client`
            // is what Google answers when the client secret is absent — the commonest
            // first-time mistake there is, because nothing pre-flights it — and it used to
            // fall into the arm below and be reported as "Signing in again will fix it".
            // Signing in again reruns the same request against the same broken registration,
            // so the app sent people round a browser consent loop that could not succeed and
            // told them to go round it again each time.
            _ if oauth::indicates_client_misconfiguration(&error) => (
                "oauthClientRejected",
                "The provider rejected Halcyon's sign-in application, not your account. Check \
                 the client ID — and, for Google, the client secret — in Settings → Accounts → \
                 Sign-in applications. Signing in again will not help until those are right."
                    .to_string(),
            ),
            _ if needs_reauth => (
                "needsReauth",
                "The saved sign-in for this account is no longer valid. Signing in again will \
                 fix it."
                    .to_string(),
            ),
            // The path named here has to be one that exists. It said "Settings → Accounts →
            // Advanced", which is a real pane — just not this one; the fields are under a
            // heading called "Sign-in applications" inside Accounts. A wrong path is worse
            // than none, because it is followed before it is doubted.
            oauth::OAuthError::NoClient { .. } => (
                "noOauthClient",
                "No sign-in application is configured for this provider yet. Add one in \
                 Settings → Accounts → Sign-in applications."
                    .to_string(),
            ),
            oauth::OAuthError::TimedOut => (
                "timedOut",
                "The browser sign-in was not completed. Starting again will reopen it.".to_string(),
            ),
            oauth::OAuthError::StateMismatch => (
                "stateMismatch",
                "The sign-in response did not match the request Halcyon started. Nothing was \
                 saved. Please try again."
                    .to_string(),
            ),
            oauth::OAuthError::Browser(_) => (
                "browser",
                "Halcyon could not open your browser to sign in.".to_string(),
            ),
            oauth::OAuthError::Refused { description, .. } => (
                "refused",
                description
                    .clone()
                    .unwrap_or_else(|| "The provider refused the sign-in.".to_string()),
            ),
            _ => (
                "network",
                "Halcyon could not reach the provider to sign in. Check your connection."
                    .to_string(),
            ),
        };

        Self {
            code: code.into(),
            message,
        }
    }
}

/// Opens a URL in the user's default browser.
///
/// docs/05 §2 requires the *system* browser and forbids an embedded WebView: Google blocks
/// embedded user agents outright, and it is also the only arrangement where the user can see
/// the address bar and know whose password box they are typing into.
///
/// `ShellExecuteW` with no verb honours the user's default browser. The URL is one this
/// process built from a provider constant plus percent-encoded parameters, never a string
/// from a page.
fn open_in_browser(url: &str) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide: Vec<u16> = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a value above 32 on success. This is the documented convention
    // and the reason the return type is an HINSTANCE that is not a handle.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The client a browser sign-in should use — refused *before* the browser opens if it cannot
/// possibly succeed.
///
/// Google rejects a Desktop client's token exchange without its secret, but only at the very
/// last step: after the user has picked an account and read a consent screen. Nothing checked
/// first, so someone who had pasted a client id and not the secret went through all of that to
/// be told `invalid_client` at the end. The sync engine has refused this case for a while
/// (`SyncError::MissingClientSecret`); the two commands that open a browser now refuse it too,
/// and say what is actually missing.
async fn client_for_sign_in(db: &Db, provider: Provider) -> Result<oauth::ClientConfig, AppError> {
    let client = db
        .read(move |conn| accounts::client_config(conn, provider))
        .await?
        .ok_or(oauth::OAuthError::NoClient {
            provider: provider.id().to_string(),
        })?;

    if provider.requires_client_secret() && client.client_secret.is_none() {
        return Err(AppError {
            code: "missingClientSecret".into(),
            message: "Google needs the client secret of your sign-in application as well as its \
                      client ID. Paste it into Settings → Accounts → Sign-in applications, then \
                      try again."
                .into(),
        });
    }

    Ok(client)
}

/// The provider picker's contents.
#[tauri::command]
pub async fn providers_list(db: State<'_, Db>) -> Response<Vec<ProviderInfo>> {
    let infos = db
        .read(|conn| {
            Ok(provider::ALL
                .iter()
                .map(|&p| {
                    let configured = accounts::client_config(conn, p).ok().flatten().is_some();
                    provider::describe(p, configured)
                })
                .collect::<Vec<_>>())
        })
        .await?;

    Ok(infos)
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    pub imap: ServerSettings,
    pub smtp: ServerSettings,
    pub source: DiscoverySource,
    /// The sentence the settings pane shows under the prefilled fields.
    pub explanation: String,
    pub needs_confirmation: bool,
    /// Set when the domain says it wants OAuth — the form switches away from a password box.
    pub suggested_provider: Option<String>,
}

impl DiscoveryResult {
    fn from(found: Discovered) -> Self {
        let suggested_provider = found.oauth_hint.as_ref().and_then(|_| {
            let host = found.imap.host.to_ascii_lowercase();
            if host.contains("google") || host.contains("gmail") {
                Some("google".to_string())
            } else if host.contains("outlook") || host.contains("office365") {
                Some("microsoft".to_string())
            } else {
                None
            }
        });

        Self {
            explanation: found.source.explain().to_string(),
            needs_confirmation: found.source.needs_confirmation(),
            imap: found.imap,
            smtp: found.smtp,
            source: found.source,
            suggested_provider,
        }
    }
}

/// Works out a domain's servers. docs/04 Phase 4 — ISPDB, autoconfig, SRV, then probing.
#[tauri::command]
pub async fn account_discover(email: String) -> Response<Option<DiscoveryResult>> {
    // A recognised address needs no lookup at all, and answering instantly is better than
    // a spinner that resolves to the same thing.
    if let Some(domain) = autodiscover::domain_of(&email) {
        let known = match domain.as_str() {
            "gmail.com" | "googlemail.com" => Some(Provider::Google),
            "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => Some(Provider::Microsoft),
            "icloud.com" | "me.com" | "mac.com" => Some(Provider::ICloud),
            "yahoo.com" | "ymail.com" | "rocketmail.com" => Some(Provider::Yahoo),
            _ => None,
        };

        if let Some(known) = known {
            let (imap, smtp) = known.servers().expect("a known provider has servers");

            return Ok(Some(DiscoveryResult {
                imap,
                smtp,
                source: DiscoverySource::Known,
                explanation: DiscoverySource::Known.explain().to_string(),
                needs_confirmation: false,
                suggested_provider: Some(known.id().to_string()),
            }));
        }
    }

    Ok(autodiscover::discover(&email)
        .await
        .map(DiscoveryResult::from))
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ServerInput {
    pub host: String,
    pub port: u16,
    /// "tls" or "starttls".
    pub security: String,
}

impl ServerInput {
    fn into_settings(self) -> ServerSettings {
        ServerSettings {
            host: self.host.trim().to_string(),
            port: self.port,
            security: if self.security.eq_ignore_ascii_case("starttls") {
                Security::StartTls
            } else {
                Security::Tls
            },
        }
    }
}

fn servers_for(
    provider: Provider,
    imap: Option<ServerInput>,
    smtp: Option<ServerInput>,
) -> Result<(ServerSettings, ServerSettings), AppError> {
    match (imap, smtp) {
        (Some(imap), Some(smtp)) => Ok((imap.into_settings(), smtp.into_settings())),
        _ => provider
            .servers()
            .ok_or_else(|| bad_request("This account needs incoming and outgoing server details.")),
    }
}

/// Tests a connection without saving anything.
///
/// The password arrives here, is used, and is dropped. It is never written, never logged and
/// never returned — the report carries steps and remedies only, and `redact_command_echo`
/// covers the one case where a server quotes the command back.
#[tauri::command]
pub async fn account_test(
    db: State<'_, Db>,
    email: String,
    provider: String,
    password: Option<String>,
    imap: Option<ServerInput>,
    smtp: Option<ServerInput>,
) -> Response<DiagnosticReport> {
    let provider = resolve(&provider)?;
    let (imap, smtp) = servers_for(provider, imap, smtp)?;

    let attempt = match provider.auth_kind() {
        AuthKind::Password => {
            Attempt::Password(Secret::new(password.ok_or_else(|| {
                bad_request("A password is needed to test this account.")
            })?))
        }
        AuthKind::OAuth2 => {
            // An OAuth account is tested with the token it already has, which means it must
            // have been through `account_add_oauth` first. Testing before signing in is not
            // a state the wizard can reach.
            let reference = credentials::reference_for(&email);
            let expiry = {
                let reference = reference.clone();
                db.read(move |conn| Ok(accounts::read_expiry(conn, &reference)))
                    .await?
            };

            let client = {
                db.read(move |conn| accounts::client_config(conn, provider))
                    .await?
                    .ok_or(oauth::OAuthError::NoClient {
                        provider: provider.id().to_string(),
                    })?
            };

            let (token, refreshed) =
                accounts::access_token(expiry, provider, &client, &reference).await?;

            if let Some(expires_at) = refreshed {
                let reference = reference.clone();
                let _ = db
                    .write(move |tx| accounts::write_expiry(tx, &reference, expires_at))
                    .await;
            }

            Attempt::OAuth {
                access_token: token,
            }
        }
    };

    Ok(verify::run(&email, provider, &imap, &smtp, attempt).await)
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AddedAccount {
    #[ts(type = "number")]
    pub id: i64,
    pub email: String,
    /// The test that ran before the account was saved.
    pub report: DiagnosticReport,
}

async fn finish_add(
    app: &AppHandle,
    db: &Db,
    account: NewAccount,
    report: DiagnosticReport,
) -> Response<AddedAccount> {
    let email = account.email.clone();

    let id = db.write(move |tx| store::insert(tx, &account)).await?;

    // The sidebar and the accounts pane both listen. docs/03 §4's event bus, not a poll.
    let _ = app.emit("accounts:changed", ());

    Ok(AddedAccount { id, email, report })
}

/// Everything the wizard collects about an account except the secret, which is a separate
/// parameter so that it is never part of a struct that could grow a `Serialize`.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    pub display_name: String,
    pub email: String,
    pub provider: String,
    pub imap: Option<ServerInput>,
    pub smtp: Option<ServerInput>,
    pub color: Option<String>,
}

/// Adds a password or app-specific-password account.
#[tauri::command]
pub async fn account_add_password(
    app: AppHandle,
    db: State<'_, Db>,
    input: AccountInput,
    password: String,
) -> Response<AddedAccount> {
    let AccountInput {
        display_name,
        email,
        provider,
        imap,
        smtp,
        color,
    } = input;

    let provider = resolve(&provider)?;
    let email = email.trim().to_lowercase();

    if autodiscover::domain_of(&email).is_none() {
        return Err(bad_request("That does not look like an email address."));
    }

    {
        let email = email.clone();
        if db
            .read(move |conn| store::exists_for_email(conn, &email))
            .await?
        {
            return Err(bad_request("That account has already been added."));
        }
    }

    let (imap, smtp) = servers_for(provider, imap, smtp)?;

    // Tested before anything is written. An account row that cannot connect is worse than
    // no row: it appears in the sidebar, fails silently, and the user has to work out why.
    let secret = Secret::new(password);
    let report = verify::run(
        &email,
        provider,
        &imap,
        &smtp,
        Attempt::Password(secret.clone()),
    )
    .await;

    if !report.ok {
        return Ok(AddedAccount {
            id: 0,
            email,
            report,
        });
    }

    // The secret goes to the Credential Manager, and only the reference to SQLite.
    let reference = credentials::reference_for(&email);
    credentials::store(&reference, Kind::Password, &secret).map_err(|error| {
        tracing::error!(%error, "could not store credential");
        AppError {
            code: "credentialStore".into(),
            message: "Windows would not save the password to Credential Manager.".into(),
        }
    })?;

    let account = NewAccount {
        display_name: if display_name.trim().is_empty() {
            email.clone()
        } else {
            display_name.trim().to_string()
        },
        email: email.clone(),
        provider,
        imap,
        smtp,
        auth_kind: AuthKind::Password,
        color,
    };

    finish_add(&app, &db, account, report).await
}

/// Adds an account by signing in through the system browser.
#[tauri::command]
pub async fn account_add_oauth(
    app: AppHandle,
    db: State<'_, Db>,
    input: AccountInput,
) -> Response<AddedAccount> {
    let AccountInput {
        display_name,
        email,
        provider,
        color,
        ..
    } = input;

    let provider = resolve(&provider)?;
    let email = email.trim().to_lowercase();

    if autodiscover::domain_of(&email).is_none() {
        return Err(bad_request("That does not look like an email address."));
    }

    {
        let email = email.clone();
        if db
            .read(move |conn| store::exists_for_email(conn, &email))
            .await?
        {
            return Err(bad_request("That account has already been added."));
        }
    }

    let client = client_for_sign_in(&db, provider).await?;

    let tokens = oauth::authorise(provider, &client, Some(&email), open_in_browser).await?;

    let reference = credentials::reference_for(&email);
    accounts::save_tokens(&reference, &tokens).map_err(|error| {
        tracing::error!(%error, "could not store tokens");
        AppError {
            code: "credentialStore".into(),
            message: "Windows would not save the sign-in to Credential Manager.".into(),
        }
    })?;

    {
        let reference = reference.clone();
        let expires_at = tokens.expires_at;
        db.write(move |tx| accounts::write_expiry(tx, &reference, expires_at))
            .await?;
    }

    let (imap, smtp) = provider
        .servers()
        .ok_or_else(|| bad_request("This provider has no known servers."))?;

    let report = verify::run(
        &email,
        provider,
        &imap,
        &smtp,
        Attempt::OAuth {
            access_token: tokens.access,
        },
    )
    .await;

    if !report.ok {
        // The tokens are kept: the sign-in itself worked, and the failure is a mailbox or
        // tenant problem the user can fix without going through the browser again.
        return Ok(AddedAccount {
            id: 0,
            email,
            report,
        });
    }

    let account = NewAccount {
        display_name: if display_name.trim().is_empty() {
            email.clone()
        } else {
            display_name.trim().to_string()
        },
        email: email.clone(),
        provider,
        imap,
        smtp,
        auth_kind: AuthKind::OAuth2,
        color,
    };

    finish_add(&app, &db, account, report).await
}

#[tauri::command]
pub async fn accounts_detail(db: State<'_, Db>) -> Response<Vec<AccountDetail>> {
    Ok(db.read(store::list).await?)
}

/// "Set it to this" and "clear it", told apart on a wire that only has `null`.
///
/// ## Why this type exists
///
/// The obvious signature is `color: Option<Option<String>>` — absent leaves the colour alone,
/// `Some(None)` clears it, `Some(Some(name))` sets it — and it was the signature here for
/// three weeks. **It cannot be deserialised from JSON.** serde resolves a `null` against the
/// *outer* option and stops, so `None` is the only thing any caller can produce for a null and
/// `Some(None)` is unreachable by construction.
///
/// The frontend tried to reach it by wrapping the value in a one-element array, which is a
/// sequence where a string was expected. So `account_update` rejected **every** colour change
/// with `invalid type: sequence, expected a string`, and had done since the feature shipped on
/// 2026-08-26: clicking a colour in Settings → Accounts did nothing, and the swatch never
/// showed as chosen because the promise that would have refreshed the list rejected first.
///
/// Nothing caught it. The failure is a rejected promise inside a webview — the command is
/// never entered, so no Rust test could see it, and the frontend never awaited the result.
///
/// An object makes the third state expressible, which is all this needs to be: absent leaves
/// the colour alone, `{"value": null}` clears it, `{"value": "green"}` sets it.
#[derive(Debug, serde::Deserialize)]
pub struct ColorChange {
    pub value: Option<String>,
}

#[tauri::command]
pub async fn account_update(
    app: AppHandle,
    db: State<'_, Db>,
    id: i64,
    display_name: Option<String>,
    color: Option<ColorChange>,
    sync_enabled: Option<bool>,
) -> Response<()> {
    db.write(move |tx| {
        store::update(
            tx,
            id,
            display_name.as_deref(),
            color.as_ref().map(|c| c.value.as_deref()),
            sync_enabled,
        )
    })
    .await?;

    let _ = app.emit("accounts:changed", ());
    Ok(())
}

#[tauri::command]
pub async fn accounts_reorder(app: AppHandle, db: State<'_, Db>, ids: Vec<i64>) -> Response<()> {
    db.write(move |tx| store::reorder(tx, &ids)).await?;

    let _ = app.emit("accounts:changed", ());
    Ok(())
}

/// Removes an account, its mail, and its secrets.
///
/// docs/04 Phase 4: *remove with purge*. All three, or the user has "removed" an account and
/// left their password in Credential Manager and their mail in the search index.
#[tauri::command]
pub async fn account_remove(app: AppHandle, db: State<'_, Db>, id: i64) -> Response<()> {
    let reference = db.write(move |tx| store::remove(tx, id)).await?;

    let Some(reference) = reference else {
        return Ok(());
    };

    // Best-effort, and reported in the log rather than to the user: the account is already
    // gone from their point of view, and an error dialog about Credential Manager at this
    // point would be about something they cannot act on.
    if let Err(error) = credentials::purge(&reference) {
        tracing::error!(%error, "could not purge credentials for a removed account");
    }

    {
        let reference = reference.clone();
        let _ = db
            .write(move |tx| accounts::forget_settings(tx, &reference))
            .await;
    }

    let _ = app.emit("accounts:changed", ());
    Ok(())
}

/// Signs in again to an existing OAuth account, keeping its mail.
///
/// The thing the re-authenticate banner has been telling people to do since Phase 4, with no
/// way to do it. `sync::engine` sets `needs_reauth` on a `Rejected` error and sends it to the
/// UI; the UI never read the flag, the banner's only button re-ran the sync that had just
/// failed, and the "Sign in again" line in Settings was a `<span>`. So an account whose refresh
/// token had expired — which for a Google client in testing mode happens every seven days —
/// could not be recovered at all except by removing the account and downloading everything
/// again.
///
/// Deliberately **not** remove-and-re-add. Only the tokens are replaced: the account row, its
/// mailboxes, its mail, its rules and its local flags all stay exactly where they are. Adding
/// the account back would re-download the mailbox and lose anything local to this machine.
///
/// The email is not taken from the caller. It comes from the stored account and is passed to
/// the provider as a login hint, so this cannot quietly re-point an account at a different
/// mailbox — sign in as someone else and the verify step below rejects it.
#[tauri::command]
pub async fn account_reauth(app: AppHandle, db: State<'_, Db>, id: i64) -> Response<()> {
    let account = db
        .read(move |conn| store::get(conn, id))
        .await?
        .ok_or_else(|| bad_request("That account no longer exists."))?;

    if account.auth_kind != AuthKind::OAuth2 {
        return Err(bad_request(
            "This account signs in with a password. Update it in its settings instead.",
        ));
    }

    let provider = resolve(&account.provider)?;
    let email = account.email.clone();

    let client = client_for_sign_in(&db, provider).await?;

    let tokens = oauth::authorise(provider, &client, Some(&email), open_in_browser).await?;

    // Checked before anything is stored. Signing in as a different person would otherwise
    // overwrite this account's credential with one for a mailbox it has none of the mail from,
    // and every later sync would then delete what is here as "missing from the server".
    let (imap, smtp) = provider
        .servers()
        .ok_or_else(|| bad_request("This provider has no known servers."))?;

    let report = verify::run(
        &email,
        provider,
        &imap,
        &smtp,
        Attempt::OAuth {
            access_token: tokens.access.clone(),
        },
    )
    .await;

    if !report.ok {
        return Err(bad_request(
            "That sign-in did not work for this account. Check you signed in as the same \
             address.",
        ));
    }

    let reference = credentials::reference_for(&email);
    accounts::save_tokens(&reference, &tokens).map_err(|error| {
        tracing::error!(%error, "could not store tokens on re-authentication");
        AppError {
            code: "credentialStore".into(),
            message: "Windows would not save the sign-in to Credential Manager.".into(),
        }
    })?;

    {
        let reference = reference.clone();
        let expires_at = tokens.expires_at;
        db.write(move |tx| accounts::write_expiry(tx, &reference, expires_at))
            .await?;
    }

    tracing::info!(account_id = id, "account re-authenticated");

    // `accounts:changed` restarts the watcher, which is what actually gets mail flowing again;
    // without it the new token sits unused until the next poll.
    let _ = app.emit("accounts:changed", ());
    Ok(())
}

/// Whether the Credential Manager still holds a usable sign-in for this account.
///
/// Returns a boolean, never the secret. Used for the re-authenticate banner.
#[tauri::command]
pub async fn account_credential_status(db: State<'_, Db>, id: i64) -> Response<bool> {
    let account = db
        .read(move |conn| store::get(conn, id))
        .await?
        .ok_or_else(|| bad_request("That account no longer exists."))?;

    Ok(account.has_credential)
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct OAuthClientStatus {
    pub provider: String,
    /// Whether this provider can be signed in to at all, from either source.
    pub configured: bool,
    /// Which application is in use, or `None` when there is none.
    pub source: Option<ClientSource>,
    /// Whether this build carries an application of its own for the provider — which is what
    /// makes clearing the user's own a "go back to Halcyon's" rather than a "turn it off".
    pub builtin: bool,
    /// The **user's own** client id, or `None` if they have not set one.
    ///
    /// Not the id in use. This fills the Settings field, and filling it with the built-in id
    /// would turn the next press of Save into an override the user never chose. The built-in
    /// id is not hidden for its own sake — it is in the address bar at every sign-in.
    pub client_id: Option<String>,
    /// Whether the user's own client has a secret stored. Never the secret itself, and never
    /// the built-in one's, which the user did not enter and cannot replace here.
    pub has_secret: bool,
    /// Whether the client **in use** lacks a secret its provider demands — the state in which
    /// every sign-in and every refresh will fail.
    ///
    /// Asked of the resolved client rather than worked out in the UI from `has_secret`,
    /// because the two disagree in one real case: a user's own client whose id is the built-in
    /// one borrows the built-in secret (`accounts::resolve_client`). The UI cannot see that id,
    /// so it would mark a working setup as broken.
    pub missing_secret: bool,
}

#[tauri::command]
pub async fn oauth_client_get(db: State<'_, Db>, provider: String) -> Response<OAuthClientStatus> {
    let provider = resolve(&provider)?;

    let resolved = db
        .read(move |conn| accounts::client_config_with_source(conn, provider))
        .await?;

    let source = resolved.as_ref().map(|(_, source)| *source);
    let custom = source == Some(ClientSource::Custom);
    let missing_secret = provider.requires_client_secret()
        && resolved
            .as_ref()
            .is_some_and(|(client, _)| client.client_secret.is_none());

    Ok(OAuthClientStatus {
        provider: provider.id().to_string(),
        configured: resolved.is_some(),
        source,
        builtin: accounts::builtin_client(provider).is_some(),
        client_id: resolved
            .filter(|_| custom)
            .map(|(client, _)| client.client_id),
        has_secret: custom && accounts::custom_client_has_secret(provider),
        missing_secret,
    })
}

/// docs/05 §2's "bring your own OAuth client" — for someone who wants their own application
/// rather than the one this build carries, or for a build that carries none.
///
/// A build from public source has nothing compiled in (docs/05 §9), and there this is the only
/// way Google and Microsoft accounts become usable at all. It also means nobody is ever
/// blocked on someone else's app-verification status or test-user list.
#[tauri::command]
pub async fn oauth_client_set(
    app: AppHandle,
    db: State<'_, Db>,
    provider: String,
    client_id: String,
    client_secret: Option<String>,
) -> Response<()> {
    let provider = resolve(&provider)?;

    if provider.auth_kind() != AuthKind::OAuth2 {
        return Err(bad_request("That provider does not sign in with OAuth."));
    }

    db.write(move |tx| {
        accounts::set_client_config(tx, provider, &client_id, client_secret.as_deref())
    })
    .await?;

    // Every other mutation in this file announced itself and this one did not, so the
    // provider list kept its cached answer and the tile stayed greyed out until the app was
    // restarted — with the client id sitting correctly in the database the whole time.
    // The most confusing kind of bug: everything worked except being told about it.
    let _ = app.emit("accounts:changed", ());

    Ok(())
}

/// Opens a provider's own setup page — Apple's app-specific password page, Yahoo's account
/// security page — in the system browser.
///
/// The URL comes from the provider table in this crate, never from the caller: a command
/// that opened an arbitrary URL on request would be a way for a hostile message to launch a
/// browser at anything it liked.
#[tauri::command]
pub async fn provider_open_setup(provider: String) -> Response<()> {
    let provider = resolve(&provider)?;

    let info = provider::describe(provider, true);
    let url = info
        .setup_url
        .ok_or_else(|| bad_request("That provider has no setup page."))?;

    open_in_browser(&url).map_err(|error| {
        tracing::warn!(%error, "could not open the browser");
        AppError {
            code: "browser".into(),
            message: "Halcyon could not open your browser.".into(),
        }
    })
}

#[cfg(test)]
mod oauth_message_tests {
    use super::*;

    fn refused(code: &str) -> AppError {
        AppError::from(oauth::OAuthError::Refused {
            provider: "google".into(),
            error: code.into(),
            description: Some("Unauthorized".into()),
        })
    }

    /// The advice has to match the remedy, and for two years' worth of first-time Google users
    /// it did not.
    ///
    /// Nothing pre-flights the client secret, so the commonest possible first failure is
    /// Google answering `invalid_client` at the token endpoint after a full browser consent
    /// round trip. That used to be reported as "The saved sign-in for this account is no
    /// longer valid. Signing in again will fix it." — which sent the user round the identical
    /// loop, against the identical broken registration, for as long as they were willing.
    #[test]
    fn a_rejected_sign_in_application_is_not_reported_as_a_dead_credential() {
        for code in ["invalid_client", "unauthorized_client"] {
            let error = refused(code);
            assert_eq!(error.code, "oauthClientRejected", "{code}");
            assert!(
                !error.message.contains("Signing in again will fix it"),
                "{code}: {}",
                error.message
            );
            assert!(error.message.contains("Sign-in applications"), "{code}");
        }

        // And the one that genuinely does mean sign in again still says so — the arm above it
        // must not have swallowed the whole class.
        let revoked = refused("invalid_grant");
        assert_eq!(revoked.code, "needsReauth");
        assert!(revoked.message.contains("Signing in again will fix it"));
    }

    /// Every path this file names has to be one the user can actually walk.
    ///
    /// `noOauthClient` said "Settings → Accounts → Advanced". Advanced is a real pane, which
    /// is what made it worse than a vague sentence: it is followed before it is doubted, and
    /// the fields are under a heading called "Sign-in applications" inside Accounts. Asserted
    /// against `panes::PANES`-style names rather than a literal so that renaming a pane to
    /// "Advanced" and moving the fields there would still have to come past this test.
    #[test]
    fn no_oauth_client_points_at_the_panel_that_holds_the_fields() {
        let error = AppError::from(oauth::OAuthError::NoClient {
            provider: "google".into(),
        });

        assert_eq!(error.code, "noOauthClient");
        assert!(
            error
                .message
                .contains("Settings → Accounts → Sign-in applications"),
            "{}",
            error.message
        );
        assert!(
            !error.message.contains("→ Advanced"),
            "Advanced is a different pane: {}",
            error.message
        );
    }

    /// No sentence Halcyon *writes* may carry protocol text, a stray newline, or run-on spacing.
    ///
    /// Deliberately not applied to the generic `Refused` arm, and that exclusion is the
    /// finding rather than a concession: that arm forwards the provider's own
    /// `error_description` verbatim, so its punctuation is the provider's business and
    /// asserting on it would be asserting on Google's copy. Every arm this file authors is
    /// held to the standard; the one arm it merely relays is checked for being a relay.
    #[test]
    fn every_sentence_this_file_writes_is_a_sentence() {
        let authored = [
            refused("invalid_client"),
            refused("unauthorized_client"),
            refused("invalid_grant"),
            AppError::from(oauth::OAuthError::NoClient {
                provider: "google".into(),
            }),
            AppError::from(oauth::OAuthError::TimedOut),
            AppError::from(oauth::OAuthError::StateMismatch),
        ];

        for error in authored {
            assert!(!error.message.is_empty());
            assert!(!error.message.contains('\n'), "{:?}", error.message);
            assert!(!error.message.contains("  "), "{:?}", error.message);
            assert!(
                error.message.ends_with('.'),
                "{:?} should end in a full stop",
                error.message
            );
            // The provider's machine-readable code is for the log, never for the banner.
            assert!(!error.message.contains("invalid_"), "{:?}", error.message);
        }

        // The relay arm: the provider's description, unaltered.
        let relayed = refused("temporarily_unavailable");
        assert_eq!(relayed.code, "refused");
        assert_eq!(relayed.message, "Unauthorized");

        // And with nothing to relay, a sentence of our own rather than an empty toast.
        let bare = AppError::from(oauth::OAuthError::Refused {
            provider: "google".into(),
            error: "temporarily_unavailable".into(),
            description: None,
        });
        assert_eq!(bare.message, "The provider refused the sign-in.");
    }
}

#[cfg(test)]
mod color_change_tests {
    use super::ColorChange;

    /// The three states, in the exact JSON the frontend sends.
    ///
    /// This is the test that would have caught the original bug in a second, and it is the
    /// reason the wire type is an object rather than a nested option.
    #[test]
    fn all_three_states_survive_the_wire() {
        let leave: Option<ColorChange> = serde_json::from_str("null").unwrap();
        assert!(leave.is_none(), "absent leaves the colour alone");

        let clear: Option<ColorChange> = serde_json::from_str(r#"{"value":null}"#).unwrap();
        assert_eq!(
            clear.map(|c| c.value),
            Some(None),
            "an explicit null clears it"
        );

        let set: Option<ColorChange> = serde_json::from_str(r#"{"value":"green"}"#).unwrap();
        assert_eq!(
            set.map(|c| c.value),
            Some(Some("green".into())),
            "a name sets it"
        );
    }

    /// The exact failure the running app reported, pinned to the signature that caused it.
    ///
    /// The frontend sent `["green"]` to reach `Some(Some(_))`. Against `Option<Option<String>>`
    /// that unwraps twice and then asks a sequence to be a string, which is the message that
    /// came back through the webview and the reason no colour was ever stored.
    ///
    /// Note the second half: this struct *would* have accepted that array, because serde will
    /// build a struct from a sequence positionally. That is recorded rather than relied on —
    /// the frontend sends the object form — but it does mean the wire is now forgiving of the
    /// shape that used to be fatal.
    #[test]
    fn the_old_signature_is_what_rejected_the_array_the_frontend_sent() {
        let old: Result<Option<Option<String>>, _> = serde_json::from_str(r#"["green"]"#);
        let message = old
            .expect_err("the old signature could not take this")
            .to_string();
        assert!(
            message.contains("invalid type: sequence, expected a string"),
            "the message the app actually printed, word for word: {message}"
        );

        let now: Option<ColorChange> = serde_json::from_str(r#"["green"]"#).unwrap();
        assert_eq!(now.map(|c| c.value), Some(Some("green".into())));
    }

    /// The signature this replaced, kept as an executable note.
    ///
    /// `Option<Option<String>>` looks like it expresses three states and expresses two: serde
    /// resolves a null against the outer option and stops, so no JSON value a caller can send
    /// produces `Some(None)`. Anyone tempted back to the nested form can run this.
    #[test]
    fn a_nested_option_cannot_express_clear_over_json() {
        let from_null: Option<Option<String>> = serde_json::from_str("null").unwrap();
        assert_eq!(from_null, None, "null resolves against the OUTER option");

        let from_value: Option<Option<String>> = serde_json::from_str(r#""green""#).unwrap();
        assert_eq!(from_value, Some(Some("green".into())));

        // There is deliberately no third case here. That is the point: there isn't one.
    }
}
