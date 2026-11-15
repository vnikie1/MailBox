//! Secrets, and the only place in this program that touches one.
//!
//! docs/03 §7 and standing rule 12: credentials live in the Windows Credential Manager,
//! never in SQLite, never in a config file, never in a log. The `account` row holds a
//! `cred_ref` — a key into this module — and nothing else.
//!
//! Everything here is written so that a secret cannot escape by accident:
//!
//! * `Secret` has no `Display`, no `Debug` that prints it, and no `Serialize`. It cannot be
//!   formatted into a log line or returned over IPC even by mistake, because the compiler
//!   refuses.
//! * Errors carry the *reference*, never the value.
//! * Nothing in this module logs at a level that includes the secret, because nothing in
//!   this module can format one.
//!
//! The verification the exit gate asks for — grep the database, the logs and the config for
//! the token and find nothing — is a property of this design, and `tests/secrets.rs`
//! asserts it against a real store rather than trusting the argument.

use keyring::Entry;

/// The Credential Manager service name every entry is filed under.
///
/// Windows shows this in Credential Manager, so it is the product name rather than
/// something internal — a user auditing their credentials should recognise it.
const SERVICE: &str = "Halcyon Mail";

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// The reference, never the secret.
    #[error("no credential stored for {reference}")]
    Missing { reference: String },

    #[error("credential store unavailable for {reference}: {source}")]
    Store {
        reference: String,
        #[source]
        source: keyring::Error,
    },
}

/// A secret in memory.
///
/// Deliberately opaque. `Debug` prints a placeholder, there is no `Display`, and there is no
/// `Serialize` — so it cannot reach a log, an error message or the IPC boundary without
/// someone writing `expose()`, which is greppable.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The plaintext. Every call site is a place a secret could leak, so this name is
    /// deliberately ugly and deliberately easy to grep for.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not the value, and not the length either — a length is a hint.
        formatter.write_str("Secret(redacted)")
    }
}

/// What kind of secret a reference points at.
///
/// Stored as distinct entries rather than one JSON blob so that revoking a refresh token
/// does not require rewriting the password, and so Credential Manager shows a user
/// something meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// IMAP/SMTP password, or an app-specific password.
    Password,
    /// OAuth refresh token — the long-lived one.
    RefreshToken,
    /// OAuth access token, cached until it expires.
    AccessToken,
    /// The OAuth client secret, when the user has brought their own client.
    ClientSecret,
}

impl Kind {
    fn suffix(self) -> &'static str {
        match self {
            Kind::Password => "password",
            Kind::RefreshToken => "refresh-token",
            Kind::AccessToken => "access-token",
            Kind::ClientSecret => "client-secret",
        }
    }
}

/// The stable key for an account's secrets.
///
/// Derived from the account's email rather than its row id, so that deleting and re-adding
/// the same account reuses the entry instead of orphaning one — Credential Manager has no
/// garbage collection, and orphaned entries accumulate forever.
pub fn reference_for(email: &str) -> String {
    format!("halcyon:{}", email.trim().to_lowercase())
}

fn entry(reference: &str, kind: Kind) -> Result<Entry, CredentialError> {
    let user = format!("{reference}:{}", kind.suffix());

    Entry::new(SERVICE, &user).map_err(|source| CredentialError::Store {
        reference: reference.to_string(),
        source,
    })
}

/// The longest secret one Credential Manager entry holds: 2,560 bytes, which keyring fills
/// with UTF-16, so 1,280 characters.
///
/// A Microsoft access token is commonly longer, so it could not be stored at all: the sign-in
/// failed with "Windows would not save the sign-in" (M29, 2026-11-14 audit). A longer secret is
/// kept in parts, each in an entry of its own, and the main entry says how many there are.
const ENTRY_CHARS: usize = 1280;

/// The size of each part, under the limit with room to spare.
const PART_CHARS: usize = 1000;

/// What the main entry of a secret kept in parts holds, before the count. A NUL cannot be typed
/// and appears in no token, so no real secret begins with it.
const PARTS_MARKER: &str = "\0halcyon-parts:";

/// More parts than any real secret needs. A count past this is not believed.
const MAX_PARTS: usize = 32;

fn part_entry(reference: &str, kind: Kind, index: usize) -> Result<Entry, CredentialError> {
    let user = format!("{reference}:{}#{index}", kind.suffix());

    Entry::new(SERVICE, &user).map_err(|source| CredentialError::Store {
        reference: reference.to_string(),
        source,
    })
}

/// How many parts a main entry's value says there are, if it is a parts header at all.
fn parts_of(value: &str) -> Option<usize> {
    value
        .strip_prefix(PARTS_MARKER)?
        .parse()
        .ok()
        .filter(|count| (1..=MAX_PARTS).contains(count))
}

/// Splits a secret into parts of at most `size` UTF-16 units, never inside a character.
fn split(value: &str, size: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut units = 0;

    for character in value.chars() {
        let width = character.len_utf16();
        if units + width > size {
            parts.push(std::mem::take(&mut current));
            units = 0;
        }
        current.push(character);
        units += width;
    }

    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Deletes the parts a stored secret was kept in, if it was. Best effort.
fn remove_parts(reference: &str, kind: Kind) {
    let Some(count) = entry(reference, kind)
        .ok()
        .and_then(|main| main.get_password().ok())
        .and_then(|value| parts_of(&value))
    else {
        return;
    };

    for index in 1..=count {
        if let Ok(part) = part_entry(reference, kind, index) {
            let _ = part.delete_credential();
        }
    }
}

pub fn store(reference: &str, kind: Kind, secret: &Secret) -> Result<(), CredentialError> {
    let failed = |source| CredentialError::Store {
        reference: reference.to_string(),
        source,
    };
    let value = secret.expose();

    // A longer secret stored before this one leaves its parts behind otherwise.
    remove_parts(reference, kind);

    if value.encode_utf16().count() <= ENTRY_CHARS && !value.starts_with(PARTS_MARKER) {
        return entry(reference, kind)?.set_password(value).map_err(failed);
    }

    let parts = split(value, PART_CHARS);
    if parts.len() > MAX_PARTS {
        return Err(failed(keyring::Error::TooLong(
            "secret".into(),
            (MAX_PARTS * PART_CHARS) as u32,
        )));
    }

    // The parts first, so the header never names parts that are not there yet.
    for (index, part) in parts.iter().enumerate() {
        part_entry(reference, kind, index + 1)?
            .set_password(part)
            .map_err(failed)?;
    }

    entry(reference, kind)?
        .set_password(&format!("{PARTS_MARKER}{}", parts.len()))
        .map_err(failed)
}

fn read(entry: &Entry, reference: &str) -> Result<String, CredentialError> {
    match entry.get_password() {
        Ok(value) => Ok(value),
        Err(keyring::Error::NoEntry) => Err(CredentialError::Missing {
            reference: reference.to_string(),
        }),
        Err(source) => Err(CredentialError::Store {
            reference: reference.to_string(),
            source,
        }),
    }
}

pub fn load(reference: &str, kind: Kind) -> Result<Secret, CredentialError> {
    let value = read(&entry(reference, kind)?, reference)?;

    let Some(count) = parts_of(&value) else {
        return Ok(Secret::new(value));
    };

    let mut whole = String::new();
    for index in 1..=count {
        whole.push_str(&read(&part_entry(reference, kind, index)?, reference)?);
    }
    Ok(Secret::new(whole))
}

/// Whether a secret of this kind exists, without reading it.
///
/// Used to decide whether an account needs re-authenticating. Reading the secret to find out
/// would pull it into memory for no reason.
pub fn exists(reference: &str, kind: Kind) -> bool {
    matches!(entry(reference, kind).map(|e| e.get_password()), Ok(Ok(_)))
}

/// Removes one secret. A missing entry is success — the caller wanted it gone.
pub fn delete(reference: &str, kind: Kind) -> Result<(), CredentialError> {
    remove_parts(reference, kind);

    match entry(reference, kind)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(source) => Err(CredentialError::Store {
            reference: reference.to_string(),
            source,
        }),
    }
}

/// Removes every secret for an account. Called when an account is removed.
///
/// Best-effort across all kinds rather than stopping at the first failure: leaving three of
/// four secrets behind because the second could not be deleted is worse than trying them
/// all and reporting.
pub fn purge(reference: &str) -> Result<(), CredentialError> {
    let mut failure = None;

    for kind in [
        Kind::Password,
        Kind::RefreshToken,
        Kind::AccessToken,
        Kind::ClientSecret,
    ] {
        if let Err(error) = delete(reference, kind) {
            failure.get_or_insert(error);
        }
    }

    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Serialises the tests that touch the real Windows Credential Manager.
///
/// Every such test in this crate shares one genuinely global resource — the signed-in user's
/// credential store — and `cargo test` runs them on several threads at once. Twice now a test
/// has written a value, read it straight back, and got the *previous* one: once a purged
/// entry that still reported as present, once an access token that read back as the version
/// before it. Neither reproduced in isolation (5 and 6 consecutive clean runs) or on demand
/// in the full suite (6 clean runs), and unique per-run entry names ruled out collisions
/// between tests.
///
/// The remaining explanation is read-after-write staleness in the store itself under
/// concurrent access from one process. That is not something this crate can fix, and it is
/// not a production risk: after `save_tokens` the caller uses the token it already holds in
/// memory rather than reading it back. It is only the tests that read immediately.
///
/// So the tests take a lock. It is not a workaround for a bug in our code; it is the honest
/// statement that these tests share a global resource and must not run concurrently.
#[cfg(test)]
pub(crate) static CREDENTIAL_STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes the lock, ignoring poisoning — a panic in one test must not cascade into every
/// other credential test reporting a lock error instead of its own result.
#[cfg(test)]
pub(crate) fn lock_store() -> std::sync::MutexGuard<'static, ()> {
    CREDENTIAL_STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A scratch credential reference that holds the store lock and cleans up **on drop**.
///
/// Cleaning up at the end of the test body is not enough, and this is not hypothetical: a
/// failing assertion unwinds past the `purge()` call, and over one working session that left
/// **fourteen** entries in the developer's real Windows Credential Manager. Nothing in the
/// suite noticed, because the check that would have caught it was a `cmdkey /list` that
/// silently errored and was read as "nothing there".
///
/// Tests write to the signed-in user's actual credential store. There is no sandbox for it.
/// So cleanup has to survive a panic, which means `Drop`.
#[cfg(test)]
pub(crate) struct Scratch {
    reference: String,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Scratch {
    /// A reference nothing else can collide with, for the life of this guard.
    ///
    /// The clock is in the name as well as the process id: Windows reuses process ids, so an
    /// id alone is not unique across runs.
    pub(crate) fn new(name: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        Self {
            reference: format!("halcyon-test:{name}:{}:{stamp}", std::process::id()),
            _lock: lock_store(),
        }
    }

    pub(crate) fn reference(&self) -> &str {
        &self.reference
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = purge(&self.reference);
    }
}

/// Borrows a **production** credential reference for the length of a test, and puts back
/// whatever was there before.
///
/// Some things cannot be tested against a scratch name: `set_client_config` derives its
/// reference from the `Provider` enum, so exercising it necessarily touches the real entry
/// for that provider. Two tests did exactly that and "cleaned up" by deleting it — which
/// silently destroyed the Google client secret of the person running the suite, on a machine
/// where they had just configured it. The tests passed. The application then failed to sign
/// in, for a reason nothing in the codebase would have explained.
///
/// So: read the old value first, restore it on drop, and delete only if there genuinely was
/// nothing there. Tests may borrow real state; they may not consume it.
#[cfg(test)]
pub(crate) struct Preserved {
    reference: String,
    kind: Kind,
    previous: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Preserved {
    pub(crate) fn new(reference: impl Into<String>, kind: Kind) -> Self {
        let reference = reference.into();
        let lock = lock_store();
        let previous = load(&reference, kind).ok().map(|s| s.expose().to_string());

        Self {
            reference,
            kind,
            previous,
            _lock: lock,
        }
    }
}

#[cfg(test)]
impl Drop for Preserved {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => {
                let _ = store(&self.reference, self.kind, &Secret::new(value));
            }
            None => {
                let _ = delete(&self.reference, self.kind);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_does_not_print_itself() {
        // The whole design rests on this: if Debug leaked, every `tracing` call that
        // included a struct containing one would leak with it.
        let secret = Secret::new("hunter2");

        assert_eq!(format!("{secret:?}"), "Secret(redacted)");
        assert!(!format!("{secret:?}").contains("hunter2"));
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn references_are_stable_and_case_insensitive() {
        // Deleting and re-adding an account must reuse the entry, or Credential Manager
        // accumulates orphans no one will ever clean up.
        assert_eq!(
            reference_for("Ada@Example.test"),
            reference_for("  ada@example.test  ")
        );
    }

    #[test]
    fn a_secret_longer_than_one_entry_is_kept_whole() {
        // M29. A Microsoft access token is commonly past the 1,280 characters one entry holds,
        // and storing it failed outright.
        let scratch = Scratch::new("long-token");
        let reference = scratch.reference();
        let token: String = (0..3_000)
            .map(|index| char::from(b'A' + (index % 26) as u8))
            .collect();

        store(reference, Kind::AccessToken, &Secret::new(token.clone())).expect("store");
        assert_eq!(
            load(reference, Kind::AccessToken).expect("load").expose(),
            token
        );
        assert!(exists(reference, Kind::AccessToken));

        // A shorter one afterwards replaces it and leaves no parts behind.
        store(reference, Kind::AccessToken, &Secret::new("short")).expect("store");
        assert_eq!(
            load(reference, Kind::AccessToken).expect("load").expose(),
            "short"
        );
        let leftover = part_entry(reference, Kind::AccessToken, 1).expect("entry");
        assert!(leftover.get_password().is_err(), "the old parts are gone");

        // And a purge takes the parts as well as the header.
        store(reference, Kind::AccessToken, &Secret::new(token)).expect("store");
        purge(reference).expect("purge");
        assert!(!exists(reference, Kind::AccessToken));
        let leftover = part_entry(reference, Kind::AccessToken, 1).expect("entry");
        assert!(leftover.get_password().is_err(), "purged with the rest");
    }

    #[test]
    fn a_secret_is_split_between_characters_never_inside_one() {
        let parts = split("ab😀cd", 3);
        assert_eq!(parts.concat(), "ab😀cd");
        assert!(parts.iter().all(|part| part.encode_utf16().count() <= 3));
        assert_eq!(parts, vec!["ab", "😀c", "d"]);
    }

    #[test]
    fn stores_loads_and_purges() {
        let scratch = Scratch::new("roundtrip");
        let reference = scratch.reference();

        store(reference, Kind::Password, &Secret::new("s3cret")).expect("store");
        assert!(exists(reference, Kind::Password));
        assert_eq!(
            load(reference, Kind::Password).expect("load").expose(),
            "s3cret"
        );

        // Kinds are separate entries: revoking a token must not disturb the password.
        store(reference, Kind::RefreshToken, &Secret::new("refresh")).expect("store");
        assert_eq!(
            load(reference, Kind::Password).expect("load").expose(),
            "s3cret"
        );

        purge(reference).expect("purge");
        assert!(!exists(reference, Kind::Password));
        assert!(!exists(reference, Kind::RefreshToken));
    }

    #[test]
    fn a_missing_secret_is_distinguishable_from_a_broken_store() {
        // The caller has to tell "needs authenticating" from "something is wrong", because
        // one prompts the user and the other is a bug.
        let scratch = Scratch::new("missing");

        match load(scratch.reference(), Kind::Password) {
            Err(CredentialError::Missing { .. }) => {}
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    #[test]
    fn deleting_something_absent_is_success() {
        let scratch = Scratch::new("absent");

        delete(scratch.reference(), Kind::AccessToken)
            .expect("delete should tolerate a missing entry");
    }

    #[test]
    fn errors_name_the_reference_and_never_the_secret() {
        let error = CredentialError::Missing {
            reference: reference_for("ada@example.test"),
        };
        let rendered = error.to_string();

        assert!(rendered.contains("ada@example.test"));
        assert!(
            !rendered.contains("password"),
            "no secret material in the message"
        );
    }
}
