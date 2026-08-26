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
            _ if needs_reauth => (
                "needsReauth",
                "The saved sign-in for this account is no longer valid. Signing in again will \
                 fix it."
                    .to_string(),
            ),
            oauth::OAuthError::NoClient { .. } => (
                "noOauthClient",
                "No sign-in application is configured for this provider yet. Add one in \
                 Settings → Accounts → Advanced."
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

    let client = db
        .read(move |conn| accounts::client_config(conn, provider))
        .await?
        .ok_or(oauth::OAuthError::NoClient {
            provider: provider.id().to_string(),
        })?;

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

#[tauri::command]
pub async fn account_update(
    app: AppHandle,
    db: State<'_, Db>,
    id: i64,
    display_name: Option<String>,
    color: Option<Option<String>>,
    sync_enabled: Option<bool>,
) -> Response<()> {
    db.write(move |tx| {
        store::update(
            tx,
            id,
            display_name.as_deref(),
            color.as_ref().map(|c| c.as_deref()),
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
    pub configured: bool,
    /// The id, which is not a secret — it is in the URL the browser is sent to. Shown so a
    /// user can confirm which client is in use.
    pub client_id: Option<String>,
    /// Whether a client secret is stored. Never the secret itself.
    pub has_secret: bool,
}

#[tauri::command]
pub async fn oauth_client_get(db: State<'_, Db>, provider: String) -> Response<OAuthClientStatus> {
    let provider = resolve(&provider)?;

    let client = db
        .read(move |conn| accounts::client_config(conn, provider))
        .await?;

    Ok(OAuthClientStatus {
        provider: provider.id().to_string(),
        configured: client.is_some(),
        client_id: client.as_ref().map(|c| c.client_id.clone()),
        has_secret: client.as_ref().is_some_and(|c| c.client_secret.is_some()),
    })
}

/// docs/05 §2's "bring your own OAuth client" mitigation.
///
/// Nothing is compiled in, so this is how Google and Microsoft accounts become usable at
/// all — and it means a user is never blocked on someone else's app-verification status.
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
