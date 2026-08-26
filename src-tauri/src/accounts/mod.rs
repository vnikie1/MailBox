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

/// Reads the configured OAuth client for a provider.
///
/// docs/05 §2 recommends offering "bring your own OAuth client", and nothing is compiled in,
/// so this returning `None` is the normal state of a fresh install rather than an error. The
/// provider picker uses it to say "this needs setting up first" instead of opening a browser
/// onto a Google error page.
pub fn client_config(
    conn: &Connection,
    provider: Provider,
) -> Result<Option<ClientConfig>, DbError> {
    let client_id: Option<String> = conn
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            params![client_id_key(provider)],
            |row| row.get(0),
        )
        .optional()?;

    let Some(client_id) = client_id.filter(|id| !id.trim().is_empty()) else {
        return Ok(None);
    };

    let client_secret =
        credentials::load(&client_secret_reference(provider), Kind::ClientSecret).ok();

    Ok(Some(ClientConfig {
        client_id,
        client_secret,
    }))
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
/// Clearing the client id is what deconfigures a provider, and that does remove the secret —
/// there is nothing left for it to belong to.
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
    fn a_fresh_install_has_no_oauth_client_and_that_is_not_an_error() {
        // Nothing is compiled in (docs/05 §2, bring your own client), so this is the normal
        // starting state. Returning an error here would make the picker show a failure on
        // first launch.
        let conn = store();

        assert!(client_config(&conn, Provider::Google)
            .expect("read")
            .is_none());
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

        // Clearing the field in the settings pane must disable the provider rather than
        // leave a blank id that opens the browser onto an error page.
        set_client_config(&conn, Provider::Google, "   ", None).expect("set");
        assert!(client_config(&conn, Provider::Google)
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
