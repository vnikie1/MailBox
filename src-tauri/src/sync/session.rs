//! Connecting to an IMAP server, authenticating, and learning what it can do.
//!
//! docs/03 §5: *Detect capability once per connection: CONDSTORE, QRESYNC, MOVE, IDLE,
//! X-GM-EXT-1, SPECIAL-USE, COMPRESS=DEFLATE, OBJECTID.*
//!
//! Capabilities are read once and carried on the session rather than asked for repeatedly.
//! They also change after authentication — servers advertise a different set to an
//! authenticated client — so the set that matters is the one read *after* login, which is a
//! detail that costs an afternoon if you read it before.
//!
//! There is no `unwrap()` in this module, per docs/06 Phase 5.

use std::time::Duration;

use async_imap::Session;
use tokio::net::TcpStream;

use crate::accounts::credentials::Secret;
use crate::accounts::provider::{Security, ServerSettings};

/// How long to wait for a server to answer at all.
///
/// Generous compared to the connection test's ten seconds: this runs in the background where
/// nobody is watching a spinner, and a slow mobile connection is not a failure.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Ceiling on the whole handshake: TLS, authentication and the capability read.
///
/// A separate, larger budget from the TCP connect, because it covers several round trips.
/// It exists because a protocol exchange really can stall forever with the socket open and
/// both ends waiting — see `XOAuth2` below. A sync that hangs is worse than one that fails:
/// a failure retries, a hang holds the account's lock and never reports anything.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

pub type TlsStream = async_native_tls::TlsStream<TcpStream>;
pub type ImapSession = Session<TlsStream>;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("could not reach {host}:{port}")]
    Unreachable {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },

    #[error("{host} did not answer in time")]
    Timeout { host: String },

    #[error("the TLS handshake with {host} failed")]
    Tls {
        host: String,
        #[source]
        source: async_native_tls::Error,
    },

    /// The account has no server settings at all — nothing to connect to.
    ///
    /// Not retryable: no amount of waiting adds an IMAP host to a row. Retrying it was the
    /// first thing the running engine got wrong, and because the accounts were synced one
    /// after another behind a single lock, three unconfigured demo accounts spent ninety
    /// seconds backing off before the real one was reached.
    #[error("{email} has no incoming mail server configured")]
    NotConfigured { email: String },

    /// The provider needs a client secret to refresh a token and none is stored.
    ///
    /// Its own variant because the remedy is specific and the obvious one is wrong: this
    /// looks exactly like a rejected sign-in, and telling the user to sign in again sends
    /// them through a browser round trip that cannot possibly help. What is missing is a
    /// field in Settings.
    #[error("no client secret is stored for {provider}")]
    MissingClientSecret { provider: String },

    /// The account is configured for a transport this engine will not use.
    ///
    /// Its own variant rather than a synthesised TLS error: refusing to connect in the clear
    /// is a policy decision (docs/05 §6), not a handshake that went wrong, and a report that
    /// conflated the two would send someone looking at their certificates.
    #[error("{host}:{port} is not an encrypted IMAP port")]
    Insecure { host: String, port: u16 },

    /// The server rejected the credential. The caller raises a re-authenticate banner rather
    /// than retrying — retrying a rejected password just locks the account out.
    #[error("{host} rejected the sign-in")]
    Rejected { host: String, detail: String },

    #[error("imap error: {0}")]
    Imap(#[from] async_imap::error::Error),

    /// A read straight off the socket, below the IMAP layer. Raw command handling reads
    /// responses itself, so its I/O errors arrive as `io::Error` rather than wrapped.
    #[error("connection error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database: {0}")]
    Db(#[from] crate::db::DbError),

    /// The mailbox's `UIDVALIDITY` changed, so every UID we hold for it is meaningless.
    /// docs/03 §5 — *drop and re-sync that mailbox. Do not try to be clever.*
    #[error("UIDVALIDITY for {mailbox} changed from {stored} to {found}")]
    UidValidityChanged {
        mailbox: String,
        stored: u32,
        found: u32,
    },

    #[error("the sync engine is shutting down")]
    ShuttingDown,
}

impl SyncError {
    /// Whether retrying could plausibly work.
    ///
    /// A rejected credential cannot be retried into success, and hammering a mail server with
    /// a password it has already refused is how an account gets locked. Everything else —
    /// network, timeout, a server having a bad minute — is worth another attempt after a
    /// backoff.
    pub fn is_retryable(&self) -> bool {
        !matches!(
            self,
            // Configuration, not weather. Waiting changes none of these.
            SyncError::Rejected { .. }
                | SyncError::NotConfigured { .. }
                | SyncError::MissingClientSecret { .. }
                | SyncError::Insecure { .. }
                | SyncError::ShuttingDown
        )
    }
}

/// What a server told us it can do. docs/03 §5.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caps {
    pub condstore: bool,
    pub qresync: bool,
    /// `MOVE` — without it a move is COPY + STORE \Deleted + EXPUNGE, which is three round
    /// trips and a window where the message exists twice.
    pub move_command: bool,
    pub idle: bool,
    /// `X-GM-EXT-1` — Gmail's thread ids, message ids and labels.
    pub gmail: bool,
    pub special_use: bool,
    pub compress: bool,
    pub objectid: bool,
    /// `UIDPLUS` — an APPEND that tells us the UID it landed on, which the outbox needs so a
    /// sent message is not fetched back as if it were new.
    pub uidplus: bool,
}

/// The wire name of one advertised capability.
///
/// Must not be `format!("{capability:?}")`. `Capability` is an enum whose atoms carry their
/// name in a payload, so the derived `Debug` renders `Atom("CONDSTORE")` — which matches
/// nothing, silently, and leaves every capability false. That is exactly what happened: IDLE,
/// CONDSTORE, MOVE and UIDPLUS were all off against a Gmail server advertising all four, and
/// nothing failed loudly because "the server cannot do this" is a legitimate answer.
fn capability_name(capability: &async_imap::types::Capability) -> String {
    use async_imap::types::Capability;

    match capability {
        Capability::Imap4rev1 => "IMAP4REV1".into(),
        // Rendered the way it is advertised, so a future `has("AUTH=XOAUTH2")` works.
        Capability::Auth(name) => format!("AUTH={name}"),
        Capability::Atom(name) => name.clone(),
    }
}

impl Caps {
    fn read(names: &[String]) -> Self {
        let has = |needle: &str| names.iter().any(|name| name.eq_ignore_ascii_case(needle));

        Self {
            condstore: has("CONDSTORE"),
            // QRESYNC implies CONDSTORE per RFC 7162, and some servers advertise only the one.
            qresync: has("QRESYNC"),
            move_command: has("MOVE"),
            idle: has("IDLE"),
            gmail: has("X-GM-EXT-1"),
            special_use: has("SPECIAL-USE"),
            compress: has("COMPRESS=DEFLATE"),
            objectid: has("OBJECTID"),
            uidplus: has("UIDPLUS"),
        }
    }

    /// Whether incremental sync can use modification sequences at all.
    pub fn has_modseq(self) -> bool {
        self.condstore || self.qresync
    }
}

/// The credential to authenticate with, already loaded from Credential Manager.
pub enum Credential {
    Password(Secret),
    OAuth(Secret),
}

/// SASL XOAUTH2, as Google and Microsoft both define it.
///
/// The `sent` flag is the whole reason this is a struct rather than a closure, and it is not
/// optional. On a **failed** XOAUTH2 exchange Google does not simply reject the command: it
/// sends a second continuation carrying a base64 JSON error, and the client is required to
/// answer that with an *empty* line before the server will send its tagged NO. A client that
/// replies to it with the credential again — which is what ignoring the challenge does —
/// leaves both ends waiting for the other, and the connection hangs indefinitely.
///
/// It did hang. A real sync against Gmail with a stale token sat on an established TLS
/// socket doing nothing at all, with no error and no log line, because the log line came
/// after the call that never returned.
struct XOAuth2 {
    email: String,
    token: String,
    sent: bool,
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        if self.sent {
            // The error continuation. An empty response is what closes the exchange and
            // lets the server send the failure it is holding.
            return String::new();
        }

        self.sent = true;

        // **Raw, not base64.** `async_imap` base64-encodes whatever this returns before
        // putting it on the wire, so encoding here sends base64 of base64 — which Gmail
        // decodes once, finds is not a SASL string, and rejects. That was the real cause of
        // the hang: the exchange never reached a state either end could complete.
        //
        // Phase 4's connection test looks different for a good reason — it writes the
        // `AUTHENTICATE XOAUTH2 <response>` line itself, so it has to do its own encoding.
        // Same protocol, different layer.
        format!("user={}\x01auth=Bearer {}\x01\x01", self.email, self.token)
    }
}

/// Opens a TLS connection and authenticates.
///
/// Certificate validation is on, with no bypass. docs/05 §6, and the same rule the Phase 4
/// connection test enforces — a sync engine that silently accepted a bad certificate would
/// undo the guarantee the setup flow made.
pub async fn connect(
    server: &ServerSettings,
    email: &str,
    credential: &Credential,
) -> Result<(ImapSession, Caps), SyncError> {
    // The whole handshake under one ceiling. Every individual step below can block, and one
    // of them provably blocks forever when a token is stale.
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(server, email, credential))
        .await
        .map_err(|_| SyncError::Timeout {
            host: server.host.clone(),
        })?
}

async fn handshake(
    server: &ServerSettings,
    email: &str,
    credential: &Credential,
) -> Result<(ImapSession, Caps), SyncError> {
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((server.host.as_str(), server.port)),
    )
    .await
    .map_err(|_| SyncError::Timeout {
        host: server.host.clone(),
    })?
    .map_err(|source| SyncError::Unreachable {
        host: server.host.clone(),
        port: server.port,
        source,
    })?;

    // Nagle off: IMAP is a request/response protocol with small commands, and coalescing a
    // 20-byte FETCH with the next one adds latency to every round trip in a backfill.
    let _ = tcp.set_nodelay(true);

    if server.security != Security::Tls {
        // Only 993/implicit TLS is supported. Phase 4's connection test says the same thing
        // in the same words, and none of the five providers needs anything else.
        return Err(SyncError::Insecure {
            host: server.host.clone(),
            port: server.port,
        });
    }

    let tls = async_native_tls::TlsConnector::new()
        .connect(server.host.as_str(), tcp)
        .await
        .map_err(|source| SyncError::Tls {
            host: server.host.clone(),
            source,
        })?;

    let mut client = async_imap::Client::new(tls);

    // **Consume the greeting.** `Client::new` does not read it, and nothing in the crate's
    // API hints that you must — but every server opens with `* OK ... ready` before it will
    // look at a command, and `authenticate`'s response loop treats the first thing it reads
    // as an answer to the command it just sent. Given the greeting instead, it waits for a
    // continuation the server has no reason to send, while the server waits for a client
    // that has stopped talking. The connection then sits open, authenticated to nothing,
    // until something times it out.
    //
    // That is precisely what a real Gmail sync did: TLS up in 40ms, then sixty seconds of
    // silence. `tests/live_gmail.rs` is what narrowed it down, by putting a boundary around
    // each step instead of around the whole handshake.
    match client.read_response().await {
        Some(Ok(_)) => {}
        Some(Err(error)) => return Err(SyncError::Io(error)),
        None => {
            return Err(SyncError::Imap(async_imap::error::Error::ConnectionLost));
        }
    }

    let mut session = match credential {
        Credential::Password(secret) => {
            client
                .login(email, secret.expose())
                .await
                .map_err(|(error, _)| SyncError::Rejected {
                    host: server.host.clone(),
                    detail: error.to_string(),
                })?
        }

        Credential::OAuth(token) => {
            let authenticator = XOAuth2 {
                email: email.to_string(),
                token: token.expose().to_string(),
                sent: false,
            };

            client
                .authenticate("XOAUTH2", authenticator)
                .await
                .map_err(|(error, _)| SyncError::Rejected {
                    host: server.host.clone(),
                    detail: error.to_string(),
                })?
        }
    };

    // Read *after* authenticating. Servers advertise a different, usually larger, set once
    // they know who is asking — CONDSTORE and X-GM-EXT-1 in particular are commonly absent
    // from the pre-login greeting.
    let capabilities = session.capabilities().await?;
    let names: Vec<String> = capabilities.iter().map(capability_name).collect();

    let caps = Caps::read(&names);

    tracing::debug!(
        host = %server.host,
        condstore = caps.condstore,
        qresync = caps.qresync,
        idle = caps.idle,
        gmail = caps.gmail,
        "imap session established"
    );

    Ok((session, caps))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_capability_is_named_by_its_atom_and_not_by_its_debug_form() {
        // The regression guard for a bug that ran from Phase 5 until it was found by reading
        // a log line. Every test below hands `Caps::read` strings written by hand, so they
        // all passed while the real path fed it `Atom("CONDSTORE")` and matched nothing.
        //
        // The lesson is the shape of the gap, not the typo: the seam between the library's
        // types and ours had no test crossing it, so both sides could be right on their own
        // terms and still not meet.
        use async_imap::types::Capability;

        let advertised = [
            Capability::Imap4rev1,
            Capability::Atom("CONDSTORE".into()),
            Capability::Atom("IDLE".into()),
            Capability::Atom("MOVE".into()),
            Capability::Atom("UIDPLUS".into()),
            Capability::Atom("X-GM-EXT-1".into()),
            Capability::Auth("XOAUTH2".into()),
        ];

        let read: Vec<String> = advertised.iter().map(capability_name).collect();
        assert_eq!(
            read,
            names(&[
                "IMAP4REV1",
                "CONDSTORE",
                "IDLE",
                "MOVE",
                "UIDPLUS",
                "X-GM-EXT-1",
                "AUTH=XOAUTH2",
            ])
        );

        // And the whole point: the flags the sync engine branches on actually come out true.
        let caps = Caps::read(&read);
        assert!(caps.condstore, "CONDSTORE did not survive the round trip");
        assert!(caps.idle, "IDLE did not survive the round trip");
        assert!(caps.move_command);
        assert!(caps.uidplus);
        assert!(caps.gmail);
        assert!(caps.has_modseq());
    }

    #[test]
    fn capabilities_are_matched_case_insensitively() {
        // Servers are inconsistent about case, and RFC 3501 says the atoms are
        // case-insensitive. Matching exactly would silently disable CONDSTORE against a
        // server that supports it, and the only symptom would be a slower sync.
        let caps = Caps::read(&names(&["Condstore", "idle", "X-Gm-Ext-1", "MOVE"]));

        assert!(caps.condstore);
        assert!(caps.idle);
        assert!(caps.gmail);
        assert!(caps.move_command);
    }

    #[test]
    fn an_empty_capability_list_disables_everything_rather_than_assuming() {
        // A server that says nothing gets the slow, universally supported path. Assuming a
        // capability that is absent produces a protocol error mid-sync.
        let caps = Caps::read(&[]);

        assert_eq!(caps, Caps::default());
        assert!(!caps.has_modseq());
    }

    #[test]
    fn qresync_alone_still_enables_the_modseq_path() {
        // RFC 7162: QRESYNC implies CONDSTORE, and some servers advertise only QRESYNC.
        // Requiring both would fall back to the windowed FLAGS scan for no reason.
        let caps = Caps::read(&names(&["QRESYNC"]));

        assert!(caps.qresync);
        assert!(caps.has_modseq());
    }

    #[test]
    fn a_gmail_capability_set_is_recognised() {
        let caps = Caps::read(&names(&[
            "IMAP4rev1",
            "UNSELECT",
            "IDLE",
            "MOVE",
            "CONDSTORE",
            "X-GM-EXT-1",
            "UIDPLUS",
            "COMPRESS=DEFLATE",
        ]));

        assert!(caps.gmail);
        assert!(caps.uidplus);
        assert!(caps.compress);
        assert!(caps.has_modseq());
        assert!(!caps.qresync, "Gmail does not offer QRESYNC");
    }

    #[test]
    fn a_capability_that_merely_contains_the_name_does_not_count() {
        // "AUTH=CONDSTORE-ISH" is not CONDSTORE. Substring matching here would enable a
        // command the server does not implement.
        let caps = Caps::read(&names(&["AUTH=PLAIN", "XCONDSTORE", "NOTIDLE"]));

        assert!(!caps.condstore);
        assert!(!caps.idle);
    }

    #[test]
    fn xoauth2_answers_the_error_continuation_with_an_empty_line() {
        // The deadlock. Gmail answers a failed XOAUTH2 with a second continuation carrying a
        // base64 error, and waits for an empty line before sending its tagged NO. Replying
        // with the credential again leaves both ends waiting and the socket open forever —
        // which is exactly what a real sync did before this flag existed.
        use async_imap::Authenticator;

        let mut authenticator = XOAuth2 {
            email: "ada@example.test".into(),
            token: "token-123".into(),
            sent: false,
        };

        let first = authenticator.process(b"");
        assert!(
            !first.is_empty(),
            "the initial response carries the credential"
        );

        let second = authenticator.process(b"eyJzdGF0dXMiOiI0MDEifQ==");
        assert_eq!(
            second, "",
            "the error continuation must be answered with nothing"
        );

        // And it stays empty however many challenges arrive.
        assert_eq!(authenticator.process(b"more"), "");
    }

    #[test]
    fn a_rejected_credential_is_not_retryable_but_a_dropped_network_is() {
        // Retrying a password the server has already refused is how an account gets locked
        // out, and the user's next symptom is a provider security email rather than an error
        // in the app.
        let rejected = SyncError::Rejected {
            host: "imap.example.test".into(),
            detail: "AUTHENTICATIONFAILED".into(),
        };
        assert!(!rejected.is_retryable());

        let unreachable = SyncError::Unreachable {
            host: "imap.example.test".into(),
            port: 993,
            source: std::io::Error::other("network is down"),
        };
        assert!(unreachable.is_retryable());

        assert!(SyncError::Timeout {
            host: "imap.example.test".into()
        }
        .is_retryable());

        assert!(!SyncError::ShuttingDown.is_retryable());
    }

    #[test]
    fn no_error_variant_can_carry_a_secret() {
        // Standing rule 12 reaches the sync engine too. These are logged on every failure.
        let rejected = SyncError::Rejected {
            host: "imap.example.test".into(),
            detail: "NO [AUTHENTICATIONFAILED] Invalid credentials".into(),
        };

        let rendered = rejected.to_string();
        assert!(rendered.contains("imap.example.test"));
        assert!(!rendered.contains("Invalid credentials"), "{rendered}");
    }

    #[tokio::test]
    async fn a_plaintext_port_is_refused_rather_than_connected_to() {
        // docs/05 §6 — no unencrypted connection to a public host, and no bypass. A sync
        // engine that quietly downgraded would undo the guarantee setup made.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();

        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let server = ServerSettings {
            host: "127.0.0.1".into(),
            port,
            security: Security::StartTls,
        };

        let result = connect(
            &server,
            "ada@example.test",
            &Credential::Password(Secret::new("x")),
        )
        .await;

        assert!(matches!(result, Err(SyncError::Insecure { .. })));
    }

    #[tokio::test]
    async fn an_unreachable_server_reports_where_it_failed() {
        let server = ServerSettings {
            host: "127.0.0.1".into(),
            port: 1,
            security: Security::Tls,
        };

        match connect(
            &server,
            "ada@example.test",
            &Credential::Password(Secret::new("x")),
        )
        .await
        {
            Err(SyncError::Unreachable { host, port, .. }) => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, 1);
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }
}
