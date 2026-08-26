//! The exit-gate check for Phase 4, as a test rather than as a one-off grep.
//!
//! docs/06 Phase 4: *after a successful setup, grep the database file, all logs and all
//! config for the password and show it appears nowhere.* Doing that by hand once proves it
//! for one build. Doing it here proves it for every build, including the one where somebody
//! adds a `tracing::debug!` with a whole struct in it.
//!
//! The sentinel is a string that could not plausibly occur by accident, so a hit is a hit.
//! Every search is over **raw bytes on disk**, not over what an API says is stored: SQLite
//! keeps freed pages and their contents until they are overwritten, and a value deleted from
//! a row can sit in the file for a long time afterwards. Checking `SELECT` results would
//! pass while the plaintext was still there to be read with a hex editor.

use std::io::Read;
use std::path::Path;

use halcyon_lib::accounts::credentials::{self, Kind, Secret};
use halcyon_lib::accounts::provider::{AuthKind, Provider};
use halcyon_lib::accounts::store::{self, NewAccount};
use halcyon_lib::db::{migrate, Db};

/// Nothing in a schema, a log format or a Windows path can produce this by chance.
const SENTINEL_PASSWORD: &str = "ZZQX-SENTINEL-PASSWORD-8f3a1c-DO-NOT-STORE";
const SENTINEL_TOKEN: &str = "ZZQX-SENTINEL-REFRESH-TOKEN-4b7e29-DO-NOT-STORE";
const SENTINEL_CLIENT_SECRET: &str = "ZZQX-SENTINEL-CLIENT-SECRET-11de60-DO-NOT-STORE";

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Every byte of a file, or an empty vector if it is not there.
fn bytes_of(path: &Path) -> Vec<u8> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };

    let mut buffer = Vec::new();
    let _ = file.read_to_end(&mut buffer);
    buffer
}

/// The database and everything SQLite writes beside it.
///
/// The `-wal` file matters as much as the main one: a value written and then rolled back
/// still passed through the write-ahead log, and WAL is on (docs/03 §3).
fn store_bytes(path: &Path) -> Vec<u8> {
    let mut all = bytes_of(path);

    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        all.extend(bytes_of(Path::new(&sidecar)));
    }

    all
}

fn unique_email() -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    format!("sentinel-{}-{stamp}@example.test", std::process::id())
}

/// Purges its reference **on drop**, so a failing assertion cannot leave a sentinel behind.
///
/// These tests write to the signed-in user's real Windows Credential Manager — there is no
/// sandbox for it. Cleaning up at the end of the test body only works when the test passes,
/// and over one working session that left fourteen orphaned entries on a real machine.
///
/// Declared here rather than shared with the unit tests because an integration test is its
/// own crate and cannot see `pub(crate)` items.
struct Scratch(String);

impl Scratch {
    fn new(email: &str) -> Self {
        Self(credentials::reference_for(email))
    }

    fn reference(&self) -> &str {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = credentials::purge(&self.0);
    }
}

#[tokio::test]
async fn a_password_account_leaves_no_trace_of_its_password_on_disk() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("halcyon.db");

    let email = unique_email();
    let scratch = Scratch::new(&email);
    let reference = scratch.reference().to_string();

    // ---- the setup a user would do -------------------------------------------------
    {
        let db = Db::open(&path).expect("open");

        // The secret goes where it belongs.
        credentials::store(&reference, Kind::Password, &Secret::new(SENTINEL_PASSWORD))
            .expect("store credential");

        let (imap, smtp) = Provider::ICloud.servers().expect("servers");
        let account = NewAccount {
            display_name: "Sentinel".into(),
            email: email.clone(),
            provider: Provider::ICloud,
            imap,
            smtp,
            auth_kind: AuthKind::Password,
            color: None,
        };

        db.write(move |tx| store::insert(tx, &account))
            .await
            .expect("insert");

        // Checkpointed so the WAL is flushed into the main file — otherwise this would be
        // testing that the sentinel is absent from a file nothing had been written to yet.
        db.read(|conn| {
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
        .await
        .expect("checkpoint");
    }

    // ---- the gate ------------------------------------------------------------------
    let bytes = store_bytes(&path);

    assert!(
        !bytes.is_empty(),
        "the database must actually exist, or this test proves nothing"
    );

    // The account is really there. Without this the assertion below would pass against an
    // empty database, which is the way this kind of test rots.
    assert!(
        contains(&bytes, &email),
        "the account row should be in the file — otherwise nothing was written"
    );

    assert!(
        !contains(&bytes, SENTINEL_PASSWORD),
        "the password appears in the database file"
    );

    // Only the reference. Standing rule 12.
    assert!(
        contains(&bytes, &reference),
        "the credential reference should be in the row"
    );

    // The guard purges on drop, including if an assertion above unwinds past this point.
}

/// The control for the test above.
///
/// A search that finds nothing is worth nothing until it has been shown to find something.
/// This writes the sentinel into the database on purpose and asserts the byte search picks
/// it up — proving the assertions above fail when they should, rather than passing because
/// SQLite happens to store text in a form the grep cannot see.
#[test]
fn the_byte_search_finds_a_secret_that_really_is_in_the_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("control.db");

    {
        let mut conn = rusqlite::Connection::open(&path).expect("open");
        migrate::run(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO setting (key, value) VALUES ('control', ?1)",
            rusqlite::params![SENTINEL_PASSWORD],
        )
        .expect("insert");

        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("checkpoint");
    }

    assert!(
        contains(&store_bytes(&path), SENTINEL_PASSWORD),
        "the search cannot see a value that is demonstrably in the file, so every \
         'not found' assertion in this file is meaningless"
    );
}

#[tokio::test]
async fn purging_an_account_takes_its_secrets_with_it() {
    let email = unique_email();
    let scratch = Scratch::new(&email);
    let reference = scratch.reference();

    credentials::store(reference, Kind::Password, &Secret::new(SENTINEL_PASSWORD)).expect("store");
    credentials::store(reference, Kind::RefreshToken, &Secret::new(SENTINEL_TOKEN)).expect("store");

    assert!(credentials::exists(reference, Kind::Password));
    assert!(credentials::exists(reference, Kind::RefreshToken));

    credentials::purge(reference).expect("purge");

    // "Remove account" that leaves the password behind is not removing the account. The
    // secrets outlive the mail otherwise, with nothing left to associate them with.
    assert!(!credentials::exists(reference, Kind::Password));
    assert!(!credentials::exists(reference, Kind::RefreshToken));
    assert!(!credentials::exists(reference, Kind::AccessToken));
    assert!(!credentials::exists(reference, Kind::ClientSecret));
}

#[test]
fn an_oauth_client_secret_never_reaches_the_settings_table() {
    let _preserved = PreservedClientSecret::new();

    // The client *id* is not a secret — it is in the URL the browser is sent to, and it is
    // shown back in Settings so a user can confirm which client is in use. The secret is a
    // secret, even though a desktop app cannot really keep one, and it goes to the same
    // place as everything else.
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("halcyon.db");

    {
        let mut conn = rusqlite::Connection::open(&path).expect("open");
        migrate::run(&mut conn).expect("migrate");

        halcyon_lib::accounts::set_client_config(
            &conn,
            Provider::Google,
            "1234.apps.googleusercontent.com",
            Some(SENTINEL_CLIENT_SECRET),
        )
        .expect("set client");

        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("checkpoint");
    }

    let bytes = store_bytes(&path);

    assert!(
        contains(&bytes, "googleusercontent"),
        "the client id should be in the settings table"
    );
    assert!(
        !contains(&bytes, SENTINEL_CLIENT_SECRET),
        "the client secret appears in the database file"
    );

    // The guard restores whatever was there before; the earlier version deleted it, and
    // wiped a real Google client secret on the machine running the suite.
}

/// Everything that formats a secret-adjacent value, checked for what it prints.
///
/// `Secret` has no `Display` and no `Serialize`, so a leak can only come from `Debug` — and
/// `Debug` is what `tracing`'s `?` sigil and `#[derive(Debug)]` on an enclosing struct both
/// reach for.
#[test]
fn nothing_that_can_be_formatted_prints_a_secret() {
    let secret = Secret::new(SENTINEL_PASSWORD);

    assert_eq!(format!("{secret:?}"), "Secret(redacted)");

    // A secret nested inside another struct is the realistic leak: nobody logs a `Secret`
    // directly, they log the thing holding it.
    #[derive(Debug)]
    #[allow(dead_code)]
    struct Holder {
        email: String,
        password: Secret,
    }

    let rendered = format!(
        "{:?}",
        Holder {
            email: "ada@example.test".into(),
            password: secret,
        }
    );

    assert!(rendered.contains("ada@example.test"));
    assert!(
        !rendered.contains(SENTINEL_PASSWORD),
        "a struct containing a Secret printed it"
    );
}

/// The error types, which are the other thing that ends up in a log.
#[test]
fn no_error_message_can_carry_a_secret() {
    use halcyon_lib::accounts::credentials::CredentialError;

    let error = CredentialError::Missing {
        reference: credentials::reference_for("ada@example.test"),
    };

    let rendered = format!("{error}");
    assert!(rendered.contains("ada@example.test"));
    assert!(!rendered.contains(SENTINEL_PASSWORD));

    // OAuth failures carry a provider id and a machine-readable code, and the description
    // is the provider's own words — never a token.
    use halcyon_lib::accounts::oauth::OAuthError;

    let refused = OAuthError::Refused {
        provider: "google".into(),
        error: "invalid_grant".into(),
        description: Some("Token has been expired or revoked.".into()),
    };

    assert!(!format!("{refused}").contains(SENTINEL_TOKEN));
    assert!(format!("{refused}").contains("invalid_grant"));
}

/// Borrows the real Google client-secret entry and puts it back afterwards.
///
/// `set_client_config` derives its reference from the provider, so this cannot be pointed at
/// a scratch name. The first version of this test "cleaned up" by deleting the entry, which
/// destroyed the client secret of the person running the suite on a machine where they had
/// just configured one — and the tests passed while doing it.
struct PreservedClientSecret {
    reference: String,
    previous: Option<String>,
}

impl PreservedClientSecret {
    fn new() -> Self {
        let reference = format!("halcyon:oauth:{}", Provider::Google.id());
        let previous = credentials::load(&reference, Kind::ClientSecret)
            .ok()
            .map(|s| s.expose().to_string());

        Self {
            reference,
            previous,
        }
    }
}

impl Drop for PreservedClientSecret {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => {
                let _ =
                    credentials::store(&self.reference, Kind::ClientSecret, &Secret::new(value));
            }
            None => {
                let _ = credentials::delete(&self.reference, Kind::ClientSecret);
            }
        }
    }
}
