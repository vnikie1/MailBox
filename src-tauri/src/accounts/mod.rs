//! Accounts, authentication and the credential store. docs/04 Phase 4, docs/05.
//!
//! The shape of this module follows one rule: **a secret is only ever in `credentials`.**
//! `store` writes rows, `oauth` obtains tokens, `verify` tests a connection — and the only
//! type any of them use to carry a secret is `credentials::Secret`, which cannot be printed,
//! serialised or sent over IPC. Standing rule 12 becomes a property of the type system
//! rather than something to remember at every call site.

pub mod autodiscover;
pub mod credentials;
pub mod oauth;
pub mod provider;
pub mod store;
pub mod verify;

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::DbError;

use credentials::{Kind, Secret};
use oauth::{ClientConfig, OAuthError, Tokens};
use provider::Provider;

/// Where a provider's OAuth client id lives.
///
/// The `setting` table, because a client id is not a secret — it appears in the URL the
/// browser is sent to. The client *secret*, when a provider issues one, goes to the
/// Credential Manager like everything else.
fn client_id_key(provider: Provider) -> String {
    format!("oauth.{}.client_id", provider.id())
}

fn client_secret_reference(provider: Provider) -> String {
    format!("halcyon:oauth:{}", provider.id())
}

/// A sign-in application compiled into this build.
///
/// ## Why this exists, reversing the Phase 4 decision
///
/// Phase 4 shipped with nothing compiled in, so Google and Microsoft were unusable on every
/// install until the user registered an application of their own — docs/05 §2 offers "bring
/// your own client" as a mitigation *for advanced users*, and the deviation made it the only
/// path. On a fresh install that meant the first thing anyone saw after choosing Google was a
/// note telling them to go and register something with Google Cloud. A mail client whose
/// Gmail support starts with that is, from the outside, a mail client without Gmail support.
///
/// ## Why it is a build input and not a constant in this file
///
/// The source is public, and docs/05 §9 is right that a secret in public source is not a
/// secret. So the values are **never in the repository**. `build.rs` reads them from the
/// environment or from the gitignored `src-tauri/oauth/clients.env`, and they reach this file
/// through `option_env!`. A build from a clean checkout has none and behaves exactly as Phase 4
/// did — which is also what docs/05 §9 asks of an open-source build. The same pattern the
/// updater's signing key already follows.
///
/// ## Why a Google client secret may be compiled in at all
///
/// Standing rule 12 keeps *secrets* out of SQLite, config, logs and error messages. A Desktop
/// OAuth client's "secret" is not one: Google issues it to installed applications knowing it
/// ships inside them, and the protection against a stolen authorisation code is PKCE, which is
/// why `oauth.rs` makes PKCE unconditional. It is still carried as `credentials::Secret` from
/// the moment it is read, so the type system keeps it out of logs and off the IPC boundary
/// exactly as it does for a user-supplied one. What rule 12 protects — the user's password and
/// tokens — is untouched: those still live only in the Credential Manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinClient {
    pub client_id: &'static str,
    pub client_secret: Option<&'static str>,
}

/// The sign-in application this build carries for a provider, if any.
///
/// `build.rs` only emits the Google pair when **both** halves are present: Google refuses every
/// token exchange from a Desktop client without its secret, so an id alone would enable the
/// provider tile, send the user through a full browser consent, and then fail at the last step.
/// No built-in client is better than a broken one.
pub fn builtin_client(provider: Provider) -> Option<BuiltinClient> {
    let (client_id, client_secret) = match provider {
        Provider::Google => (
            option_env!("HALCYON_GOOGLE_CLIENT_ID"),
            option_env!("HALCYON_GOOGLE_CLIENT_SECRET"),
        ),
        Provider::Microsoft => (option_env!("HALCYON_MICROSOFT_CLIENT_ID"), None),
        _ => (None, None),
    };

    builtin_from(provider, client_id, client_secret)
}

/// The rules `builtin_client` applies, over values that are not fixed at compile time.
///
/// `option_env!` is resolved when the crate is built, so no test can vary it — which would
/// leave the one rule here that protects users, "never half a Google client", untestable.
fn builtin_from(
    provider: Provider,
    client_id: Option<&'static str>,
    client_secret: Option<&'static str>,
) -> Option<BuiltinClient> {
    let client_id = client_id.map(str::trim).filter(|id| !id.is_empty())?;
    let client_secret = client_secret.map(str::trim).filter(|s| !s.is_empty());

    // Belt and braces behind `build.rs`, which already refuses to emit half a Google pair: a
    // value can also arrive through a stale `rustc-env` left by an earlier build script run.
    if provider.requires_client_secret() && client_secret.is_none() {
        return None;
    }

    Some(BuiltinClient {
        client_id,
        // A Microsoft public client is *rejected* if it sends a secret, so one is never passed
        // on for a provider that does not use them, whatever the build was given.
        client_secret: client_secret.filter(|_| provider.requires_client_secret()),
    })
}

/// Where the client in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum ClientSource {
    /// The one compiled into this build.
    Builtin,
    /// One the user registered and pasted into Settings. Always wins over the built-in.
    Custom,
}

/// Reads the OAuth client for a provider: the user's own if they have set one, otherwise the
/// one compiled into this build, otherwise nothing.
///
/// `None` is not an error. It is the normal state of a build from public source, and the
/// provider picker uses it to say "this needs setting up first" instead of opening a browser
/// onto a Google error page.
pub fn client_config(
    conn: &Connection,
    provider: Provider,
) -> Result<Option<ClientConfig>, DbError> {
    Ok(resolve_client(conn, provider, builtin_client(provider))?.map(|(client, _)| client))
}

/// `client_config`, plus which of the two sources answered. Settings needs the second half to
/// say "Halcyon's own application is in use" rather than showing an empty field as a fault.
pub fn client_config_with_source(
    conn: &Connection,
    provider: Provider,
) -> Result<Option<(ClientConfig, ClientSource)>, DbError> {
    resolve_client(conn, provider, builtin_client(provider))
}

/// The resolution itself, with the built-in client passed in rather than read.
///
/// Separated for one reason: `builtin_client` depends on the machine that compiled the crate.
/// Every test that asserted "a fresh install has no client" would pass on a clean checkout and
/// fail on the developer's own machine, where `clients.env` exists — a suite whose result
/// depends on a gitignored file is not a suite. Tests call this with the built-in they mean.
pub fn resolve_client(
    conn: &Connection,
    provider: Provider,
    builtin: Option<BuiltinClient>,
) -> Result<Option<(ClientConfig, ClientSource)>, DbError> {
    let custom_id: Option<String> = conn
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            params![client_id_key(provider)],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(client_id) = custom_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    {
        let mut client_secret =
            credentials::load(&client_secret_reference(provider), Kind::ClientSecret).ok();

        // A user who pasted the very id this build already carries, and no secret, has not
        // chosen a different application — they have typed the same one in again, which the
        // old setup instructions told them to do. The built-in secret belongs to that id, so
        // it is used rather than failing the refresh for want of a secret that is right here.
        //
        // Only for an *identical* id. A different id with the built-in secret would be a
        // guaranteed `invalid_client`: a client secret is only valid for the client it was
        // issued to, which is why a custom client never otherwise borrows from the built-in.
        if client_secret.is_none() {
            if let Some(builtin) = builtin.filter(|b| b.client_id == client_id) {
                client_secret = builtin.client_secret.map(Secret::new);
            }
        }

        return Ok(Some((
            ClientConfig {
                client_id,
                client_secret,
            },
            ClientSource::Custom,
        )));
    }

    Ok(builtin.map(|builtin| {
        (
            ClientConfig {
                client_id: builtin.client_id.to_string(),
                client_secret: builtin.client_secret.map(Secret::new),
            },
            ClientSource::Builtin,
        )
    }))
}

/// Whether the user's own client has a secret stored — never whether the built-in one has.
///
/// Settings uses this to say "a secret is saved". Answering from the resolved client instead
/// would report the built-in secret as the user's, and invite them to "replace" something they
/// never entered.
pub fn custom_client_has_secret(provider: Provider) -> bool {
    credentials::exists(&client_secret_reference(provider), Kind::ClientSecret)
}

/// Stores an OAuth client. The id goes to `setting`; the secret, if any, to the Credential
/// Manager.
///
/// **An absent or empty `client_secret` means "keep the one already stored", not "delete
/// it".** The settings pane says so in as many words — *"A secret is saved. Type a new one to
/// replace it."* — and a password field cannot be prefilled with what is already there, so
/// the box is empty every time the pane is opened. Treating empty as "clear" therefore
/// destroyed the secret the moment anyone edited the client id and saved, which is a thing
/// people do. The account then failed to sign in with nothing on screen to explain why.
///
/// Clearing the client id removes the user's own client, and its secret with it — there is
/// nothing left for the secret to belong to. In a build that carries a built-in client that is
/// "go back to Halcyon's application", not "disable the provider": `resolve_client` falls
/// through to the built-in. It used to be the second, and clearing the field stopped every
/// OAuth account at once.
pub fn set_client_config(
    conn: &Connection,
    provider: Provider,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<(), DbError> {
    let client_id = client_id.trim();

    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![client_id_key(provider), client_id],
    )?;

    let reference = client_secret_reference(provider);

    if client_id.is_empty() {
        let _ = credentials::delete(&reference, Kind::ClientSecret);
        return Ok(());
    }

    if let Some(secret) = client_secret.map(str::trim).filter(|s| !s.is_empty()) {
        // A failure here is not fatal to the row above: the id is still correct, and the
        // sign-in will report a missing secret rather than a corrupt configuration.
        let _ = credentials::store(&reference, Kind::ClientSecret, &Secret::new(secret));
    }

    Ok(())
}

/// Persists a token set for an account.
///
/// The refresh token is only written when the provider sent one. Overwriting a good refresh
/// token with nothing is how an account silently logs itself out a few hours later: most
/// providers omit it on a refresh, meaning "keep the one you have".
pub fn save_tokens(reference: &str, tokens: &Tokens) -> Result<(), credentials::CredentialError> {
    credentials::store(reference, Kind::AccessToken, &tokens.access)?;

    if let Some(refresh) = &tokens.refresh {
        credentials::store(reference, Kind::RefreshToken, refresh)?;
    }

    Ok(())
}

/// A valid access token for an account, refreshing it first if it is close to expiring.
///
/// The expiry is kept in `setting` rather than beside the token: Credential Manager entries
/// are for secrets, and an expiry timestamp is not one — putting it there would mean a
/// keyring read on every request just to check a clock.
pub async fn access_token(
    conn_expiry: i64,
    provider: Provider,
    client: &ClientConfig,
    reference: &str,
) -> Result<(Secret, Option<i64>), OAuthError> {
    if !oauth::needs_refresh(conn_expiry) {
        if let Ok(token) = credentials::load(reference, Kind::AccessToken) {
            return Ok((token, None));
        }
    }

    let refresh_token = credentials::load(reference, Kind::RefreshToken).map_err(|_| {
        // No refresh token means there is nothing to refresh from, and the only honest
        // answer is that the user has to sign in again.
        OAuthError::Refused {
            provider: provider.id().to_string(),
            error: "invalid_grant".into(),
            description: Some("no refresh token is stored for this account".into()),
        }
    })?;

    let tokens = oauth::refresh(provider, client, &refresh_token).await?;
    let expires_at = tokens.expires_at;

    let _ = save_tokens(reference, &tokens);

    Ok((tokens.access, Some(expires_at)))
}

fn expiry_key(reference: &str) -> String {
    format!("oauth.expiry.{reference}")
}

pub fn read_expiry(conn: &Connection, reference: &str) -> i64 {
    conn.query_row(
        "SELECT value FROM setting WHERE key = ?1",
        params![expiry_key(reference)],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|value| value.parse().ok())
    .unwrap_or(0)
}

pub fn write_expiry(conn: &Connection, reference: &str, expires_at: i64) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![expiry_key(reference), expires_at.to_string()],
    )?;

    Ok(())
}

/// Clears the settings rows an account leaves behind. Called alongside `credentials::purge`.
pub fn forget_settings(conn: &Connection, reference: &str) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM setting WHERE key = ?1",
        params![expiry_key(reference)],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;

    fn store() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open");
        migrate::run(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn a_build_with_nothing_compiled_in_has_no_oauth_client_and_that_is_not_an_error() {
        // A build from public source carries no client (docs/05 §9), so this is its normal
        // starting state. Returning an error here would make the picker show a failure on
        // first launch.
        //
        // Through `resolve_client` with `None`, not `client_config`: the latter reads what
        // *this* machine compiled in, and on the developer's own machine that is a client.
        let conn = store();

        assert!(resolve_client(&conn, Provider::Google, None)
            .expect("read")
            .is_none());
    }

    const BUILTIN_GOOGLE: BuiltinClient = BuiltinClient {
        client_id: "builtin.apps.googleusercontent.com",
        client_secret: Some("GOCSPX-builtin-test-secret"),
    };

    #[test]
    fn a_built_in_client_is_used_when_the_user_has_not_set_one() {
        // The reason this exists. A build that carries a client must not greet a new user with
        // "register your own sign-in application" — the tile has to just work.
        let conn = store();

        let (client, source) = resolve_client(&conn, Provider::Google, Some(BUILTIN_GOOGLE))
            .expect("read")
            .expect("the built-in client should be offered");

        assert_eq!(source, ClientSource::Builtin);
        assert_eq!(client.client_id, "builtin.apps.googleusercontent.com");
        assert_eq!(
            client.client_secret.as_ref().map(Secret::expose),
            Some("GOCSPX-builtin-test-secret")
        );
    }

    #[test]
    fn the_users_own_client_always_wins_over_the_built_in() {
        let _preserved = credentials::Preserved::new(
            client_secret_reference(Provider::Google),
            Kind::ClientSecret,
        );
        let conn = store();

        set_client_config(
            &conn,
            Provider::Google,
            "mine.apps.googleusercontent.com",
            Some("GOCSPX-mine"),
        )
        .expect("set");

        let (client, source) = resolve_client(&conn, Provider::Google, Some(BUILTIN_GOOGLE))
            .expect("read")
            .expect("configured");

        assert_eq!(source, ClientSource::Custom);
        assert_eq!(client.client_id, "mine.apps.googleusercontent.com");
        assert_eq!(
            client.client_secret.as_ref().map(Secret::expose),
            Some("GOCSPX-mine"),
            "a custom client must use its own secret, never the built-in one"
        );
    }

    #[test]
    fn clearing_your_own_client_goes_back_to_the_built_in_rather_than_disabling_the_provider() {
        // With nothing compiled in, clearing the field disables the provider. With a built-in it
        // must not: that used to stop every OAuth account on the machine at once, and in a build
        // that has a perfectly good client to fall back to there is no reason for it.
        let _preserved = credentials::Preserved::new(
            client_secret_reference(Provider::Google),
            Kind::ClientSecret,
        );
        let conn = store();

        set_client_config(
            &conn,
            Provider::Google,
            "mine.apps.googleusercontent.com",
            None,
        )
        .expect("set");
        set_client_config(&conn, Provider::Google, "", None).expect("clear");

        let (client, source) = resolve_client(&conn, Provider::Google, Some(BUILTIN_GOOGLE))
            .expect("read")
            .expect("the built-in client should take over again");

        assert_eq!(source, ClientSource::Builtin);
        assert_eq!(client.client_id, BUILTIN_GOOGLE.client_id);
    }

    #[test]
    fn retyping_the_built_in_id_without_a_secret_uses_the_built_in_secret() {
        // The old setup instructions told people to paste their client id into Settings. Someone
        // who does that in a build carrying the same client, and leaves the secret box empty, has
        // not chosen a different application — failing their refresh for want of a secret the
        // build already holds would be our fault, not theirs.
        let reference = client_secret_reference(Provider::Google);
        let _preserved = credentials::Preserved::new(reference.clone(), Kind::ClientSecret);
        let _ = credentials::delete(&reference, Kind::ClientSecret);
        let conn = store();

        set_client_config(&conn, Provider::Google, BUILTIN_GOOGLE.client_id, None).expect("set");

        let (client, source) = resolve_client(&conn, Provider::Google, Some(BUILTIN_GOOGLE))
            .expect("read")
            .expect("configured");

        assert_eq!(source, ClientSource::Custom);
        assert_eq!(
            client.client_secret.as_ref().map(Secret::expose),
            BUILTIN_GOOGLE.client_secret
        );
    }

    #[test]
    fn a_different_id_never_borrows_the_built_in_secret() {
        // A client secret is only valid for the client it was issued to. Lending the built-in
        // one to another id would turn "you have not entered a secret" — which the sync engine
        // explains properly — into an `invalid_client` from Google, which explains nothing.
        let reference = client_secret_reference(Provider::Google);
        let _preserved = credentials::Preserved::new(reference.clone(), Kind::ClientSecret);
        let _ = credentials::delete(&reference, Kind::ClientSecret);
        let conn = store();

        set_client_config(
            &conn,
            Provider::Google,
            "other.apps.googleusercontent.com",
            None,
        )
        .expect("set");

        let (client, _) = resolve_client(&conn, Provider::Google, Some(BUILTIN_GOOGLE))
            .expect("read")
            .expect("configured");

        assert!(client.client_secret.is_none());
    }

    #[test]
    fn half_a_google_client_is_no_client_at_all() {
        // An id without its secret would light up the Google tile, send the user through a whole
        // browser consent, and then fail at the token exchange. Nothing is better than that.
        assert_eq!(
            builtin_from(Provider::Google, Some("x.apps.googleusercontent.com"), None),
            None
        );
        assert_eq!(
            builtin_from(
                Provider::Google,
                Some("x.apps.googleusercontent.com"),
                Some("  ")
            ),
            None,
            "a blank secret is no secret"
        );
        assert_eq!(builtin_from(Provider::Google, None, Some("GOCSPX-x")), None);
        assert_eq!(
            builtin_from(Provider::Google, Some("   "), Some("GOCSPX-x")),
            None
        );

        assert_eq!(
            builtin_from(
                Provider::Google,
                Some(" x.apps.googleusercontent.com "),
                Some(" GOCSPX-x ")
            ),
            Some(BuiltinClient {
                client_id: "x.apps.googleusercontent.com",
                client_secret: Some("GOCSPX-x"),
            }),
            "whitespace from a hand-edited file is trimmed rather than sent to Google"
        );
    }

    #[test]
    fn a_microsoft_built_in_never_carries_a_secret() {
        // Microsoft public clients have no secret and reject a request that sends one. Needing
        // no secret is also what makes a Microsoft client safe to build in unconditionally.
        let id = "00000000-0000-0000-0000-000000000000";

        assert_eq!(
            builtin_from(Provider::Microsoft, Some(id), None),
            Some(BuiltinClient {
                client_id: id,
                client_secret: None,
            })
        );
        assert_eq!(
            builtin_from(Provider::Microsoft, Some(id), Some("should-never-be-sent")),
            Some(BuiltinClient {
                client_id: id,
                client_secret: None,
            })
        );
    }

    #[test]
    fn providers_that_sign_in_with_a_password_never_have_a_built_in_client() {
        // `builtin_client` is what keeps them out — `builtin_from` is only ever handed values
        // for an OAuth provider, so the guard that matters is the match arm above it.
        for provider in [Provider::ICloud, Provider::Yahoo, Provider::Other] {
            assert_eq!(builtin_client(provider), None, "{provider:?}");
        }
    }

    #[test]
    fn the_file_holding_the_built_in_client_is_never_committed() {
        // It holds a Google client secret, and the repository is public (docs/05 §9). Three
        // ways that could go wrong, each checked: the ignore rule removed, the committed template
        // filled in by someone who copied the wrong file, or the real file force-added.
        //
        // Paths are relative to src-tauri/, which is where cargo runs tests.
        let gitignore = std::fs::read_to_string("../.gitignore").expect("read .gitignore");
        assert!(
            gitignore
                .lines()
                .any(|line| line.trim() == "src-tauri/oauth/clients.env"),
            "src-tauri/oauth/clients.env must be listed in .gitignore"
        );

        let example =
            std::fs::read_to_string("oauth/clients.env.example").expect("read the template");
        for line in example
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let (name, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("`{line}` is not NAME=value"));
            assert!(
                value.trim().is_empty(),
                "{name} has a value in clients.env.example, which is committed"
            );
        }

        // `--error-unmatch` exits 0 only for a tracked path. Skipped where git is not installed;
        // outside a work tree git exits non-zero, which correctly passes.
        if let Ok(output) = std::process::Command::new("git")
            .args(["ls-files", "--error-unmatch", "oauth/clients.env"])
            .output()
        {
            assert!(
                !output.status.success(),
                "src-tauri/oauth/clients.env is tracked by git — remove it with \
                 `git rm --cached` and rotate the Google client secret"
            );
        }
    }

    #[test]
    fn a_client_id_round_trips_and_an_empty_one_reads_as_absent() {
        // Borrows the real Google entry rather than consuming it — see `Preserved`.
        let _preserved = credentials::Preserved::new(
            client_secret_reference(Provider::Google),
            Kind::ClientSecret,
        );

        let conn = store();

        set_client_config(
            &conn,
            Provider::Google,
            "123.apps.googleusercontent.com",
            None,
        )
        .expect("set");

        let client = client_config(&conn, Provider::Google)
            .expect("read")
            .expect("configured");
        assert_eq!(client.client_id, "123.apps.googleusercontent.com");

        // With nothing compiled in, clearing the field must disable the provider rather than
        // leave a blank id that opens the browser onto an error page.
        set_client_config(&conn, Provider::Google, "   ", None).expect("set");
        assert!(resolve_client(&conn, Provider::Google, None)
            .expect("read")
            .is_none());
    }

    #[test]
    fn the_client_id_is_in_the_database_and_the_client_secret_is_not() {
        // The exit gate greps the database file for secret material. The id is not a
        // secret — it is in the URL the browser is sent to — but the secret must not be
        // in any row.
        //
        // `set_client_config` derives its reference from the provider, so this test cannot
        // avoid touching the real Google entry. It borrows it and puts it back: the earlier
        // version deleted it instead, and destroyed a real client secret on the machine of
        // the person running the suite.
        let _preserved = credentials::Preserved::new(
            client_secret_reference(Provider::Google),
            Kind::ClientSecret,
        );

        let conn = store();

        set_client_config(
            &conn,
            Provider::Google,
            "123.apps.googleusercontent.com",
            Some("GOCSPX-do-not-store-me"),
        )
        .expect("set");

        let mut statement = conn
            .prepare("SELECT key, value FROM setting")
            .expect("prepare");
        let rows: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("rows");

        assert!(rows
            .iter()
            .any(|(_, value)| value.contains("googleusercontent")));
        assert!(
            !rows.iter().any(|(_, value)| value.contains("GOCSPX")),
            "the client secret must never reach SQLite"
        );

        // No cleanup here: `_preserved` restores whatever was there before, on drop.
    }

    #[test]
    fn a_client_written_in_a_transaction_is_visible_immediately() {
        // `oauth_client_set` used to run this through the *reader* pool, bypassing the single
        // writer docs/03 §3 mandates. It worked, which is why it survived review — but it
        // is the shape that produces SQLITE_BUSY under concurrency, and the writer actor
        // exists precisely so nobody has to think about that.
        let mut conn = store();

        {
            let tx = conn.transaction().expect("tx");
            set_client_config(
                &tx,
                Provider::Google,
                "abc.apps.googleusercontent.com",
                None,
            )
            .expect("set");
            tx.commit().expect("commit");
        }

        let client = client_config(&conn, Provider::Google)
            .expect("read")
            .expect("configured");

        assert_eq!(client.client_id, "abc.apps.googleusercontent.com");
    }

    #[test]
    fn saving_a_client_id_again_does_not_wipe_the_stored_secret() {
        // The settings pane says "A secret is saved. Type a new one to replace it." — and a
        // password field cannot be prefilled, so the box is empty every time it is opened.
        // Treating that as "clear it" destroyed the secret whenever anyone edited the client
        // id and saved, and the account then failed to sign in with nothing explaining why.
        let _preserved = credentials::Preserved::new(
            client_secret_reference(Provider::Google),
            Kind::ClientSecret,
        );
        let conn = store();
        let reference = client_secret_reference(Provider::Google);

        set_client_config(&conn, Provider::Google, "id-1", Some("the-secret")).expect("set");
        assert!(credentials::exists(&reference, Kind::ClientSecret));

        // Saving a new id with the secret box left empty.
        set_client_config(&conn, Provider::Google, "id-2", None).expect("set");

        assert_eq!(
            credentials::load(&reference, Kind::ClientSecret)
                .expect("the secret must survive")
                .expose(),
            "the-secret"
        );

        // Clearing the id deconfigures the provider, and that does take the secret with it —
        // there is nothing left for it to belong to.
        set_client_config(&conn, Provider::Google, "", None).expect("set");
        assert!(!credentials::exists(&reference, Kind::ClientSecret));
    }

    #[test]
    fn an_expiry_is_a_setting_not_a_credential() {
        // Storing it in Credential Manager would mean a keyring read on every request just
        // to look at a clock.
        let conn = store();

        assert_eq!(read_expiry(&conn, "halcyon:ada@example.test"), 0);

        write_expiry(&conn, "halcyon:ada@example.test", 1_800_000_000).expect("write");
        assert_eq!(
            read_expiry(&conn, "halcyon:ada@example.test"),
            1_800_000_000
        );

        forget_settings(&conn, "halcyon:ada@example.test").expect("forget");
        assert_eq!(read_expiry(&conn, "halcyon:ada@example.test"), 0);
    }

    #[test]
    fn saving_a_refreshed_token_set_does_not_erase_the_refresh_token() {
        // Providers usually omit the refresh token on a refresh, meaning "keep yours". A
        // save that blanked it would log the account out a few hours later, and the cause
        // would be invisible.
        //
        // The guard holds the store lock and purges on drop, so the failing assertion this
        // test once produced cannot leave an entry behind in a real Credential Manager.
        let scratch = credentials::Scratch::new("tokens");
        let reference = scratch.reference();

        let first = Tokens {
            access: Secret::new("access-1"),
            refresh: Some(Secret::new("refresh-1")),
            expires_at: 1_000,
        };
        save_tokens(reference, &first).expect("save");

        let second = Tokens {
            access: Secret::new("access-2"),
            refresh: None,
            expires_at: 2_000,
        };
        save_tokens(reference, &second).expect("save");

        assert_eq!(
            credentials::load(reference, Kind::AccessToken)
                .expect("access")
                .expose(),
            "access-2"
        );
        assert_eq!(
            credentials::load(reference, Kind::RefreshToken)
                .expect("refresh")
                .expose(),
            "refresh-1",
            "the refresh token must survive a refresh that did not return one"
        );

        // No explicit purge: the guard does it on drop, including when an assertion above
        // unwinds past this point.
    }
}
