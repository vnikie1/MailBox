//! The connection test, and the diagnostic report it produces.
//!
//! docs/04 Phase 4: *a connection test with a readable diagnostic report, not
//! "authentication failed"*. That sentence is the whole reason this module is more than
//! twenty lines. Every mail client on Windows can tell a user that authentication failed;
//! almost none can tell them **which** of the six things that could mean it was, and that is
//! the difference between a user who fixes their account and a user who gives up.
//!
//! So the test runs as a sequence of named steps — resolve, connect, TLS, greeting,
//! capabilities, authenticate, open INBOX — each reporting pass, fail or skipped, and a
//! failure carries the server's own words plus a remedy in plain English.
//!
//! The protocols are spoken directly rather than through `async-imap` (which arrives with
//! the sync engine in Phase 5). A diagnostic wants the raw response line — that is the
//! evidence — and a session-oriented client is built to hide exactly that.

use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use ts_rs::TS;

use super::credentials::Secret;
use super::provider::{AuthKind, Provider, Security, ServerSettings};

/// Per-step budget. A mail server that has not answered in ten seconds is not going to.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Passed,
    Failed,
    /// Not reached, because an earlier step failed. Shown greyed rather than hidden — a
    /// report that silently stops halfway looks like the app gave up.
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CheckStep {
    pub name: String,
    pub status: StepStatus,
    /// What happened, in plain English.
    pub detail: String,
    /// What the user can do about it. Only ever set on a failure.
    pub remedy: Option<String>,
    /// The server's own response, when there was one. Shown behind a disclosure triangle:
    /// useless to most users, and the only thing that helps when someone has to ask their
    /// administrator.
    pub server_said: Option<String>,
    #[ts(type = "number")]
    pub elapsed_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub ok: bool,
    pub imap: Vec<CheckStep>,
    pub smtp: Vec<CheckStep>,
    /// One line for the top of the sheet — the first failure's remedy, or a confirmation.
    pub summary: String,
}

impl DiagnosticReport {
    fn build(imap: Vec<CheckStep>, smtp: Vec<CheckStep>) -> Self {
        let first_failure = imap
            .iter()
            .chain(smtp.iter())
            .find(|step| step.status == StepStatus::Failed);

        let (ok, summary) = match first_failure {
            None => (
                true,
                "Halcyon connected to both servers and signed in successfully.".to_string(),
            ),
            Some(step) => (
                false,
                step.remedy.clone().unwrap_or_else(|| step.detail.clone()),
            ),
        };

        Self {
            ok,
            imap,
            smtp,
            summary,
        }
    }
}

/// Collects steps and stops caring once one has failed.
struct Report {
    steps: Vec<CheckStep>,
    failed: bool,
}

impl Report {
    fn new() -> Self {
        Self {
            steps: Vec::new(),
            failed: false,
        }
    }

    fn passed(&mut self, name: &str, detail: impl Into<String>, started: Instant) {
        self.steps.push(CheckStep {
            name: name.into(),
            status: StepStatus::Passed,
            detail: detail.into(),
            remedy: None,
            server_said: None,
            elapsed_ms: started.elapsed().as_millis() as u32,
        });
    }

    fn failed(
        &mut self,
        name: &str,
        detail: impl Into<String>,
        remedy: Option<String>,
        server_said: Option<String>,
        started: Instant,
    ) {
        self.failed = true;
        self.steps.push(CheckStep {
            name: name.into(),
            status: StepStatus::Failed,
            detail: detail.into(),
            remedy,
            server_said,
            elapsed_ms: started.elapsed().as_millis() as u32,
        });
    }

    fn skip(&mut self, name: &str, reason: &str) {
        self.steps.push(CheckStep {
            name: name.into(),
            status: StepStatus::Skipped,
            detail: reason.into(),
            remedy: None,
            server_said: None,
            elapsed_ms: 0,
        });
    }
}

/// A connection that may or may not have been upgraded to TLS yet.
enum Stream {
    Plain(TcpStream),
    Tls(Box<async_native_tls::TlsStream<TcpStream>>),
}

impl Stream {
    async fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        let bytes = format!("{line}\r\n");
        match self {
            Stream::Plain(stream) => stream.write_all(bytes.as_bytes()).await,
            Stream::Tls(stream) => stream.write_all(bytes.as_bytes()).await,
        }
    }

    async fn read_some(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(stream) => stream.read(buffer).await,
            Stream::Tls(stream) => stream.read(buffer).await,
        }
    }

    /// Reads until `is_complete` says the response has ended, or the budget runs out.
    ///
    /// Both protocols here are line-based with a terminator the caller recognises, so the
    /// predicate is passed in rather than duplicating the read loop twice.
    async fn read_response(
        &mut self,
        is_complete: impl Fn(&str) -> bool,
    ) -> Result<String, String> {
        let mut accumulated = String::new();
        let mut buffer = [0u8; 4096];

        loop {
            let read = tokio::time::timeout(STEP_TIMEOUT, self.read_some(&mut buffer)).await;

            match read {
                Err(_) => return Err("the server stopped responding".into()),
                Ok(Err(error)) => return Err(error.to_string()),
                Ok(Ok(0)) => {
                    return if accumulated.is_empty() {
                        Err("the server closed the connection without saying anything".into())
                    } else {
                        Ok(accumulated)
                    }
                }
                Ok(Ok(count)) => {
                    accumulated.push_str(&String::from_utf8_lossy(&buffer[..count]));

                    if is_complete(&accumulated) {
                        return Ok(accumulated);
                    }

                    // A server that keeps talking without ever completing a response would
                    // otherwise grow this string until the process died.
                    if accumulated.len() > 64 * 1024 {
                        return Err("the server sent an unreasonably long response".into());
                    }
                }
            }
        }
    }
}

async fn tls_upgrade(stream: TcpStream, host: &str) -> Result<Stream, async_native_tls::Error> {
    // No `danger_accept_invalid_certs`, and no option to add one. docs/05 §6: certificate
    // validation is on for public hosts with no user-visible bypass. On Windows this is
    // SChannel, so it uses the system trust store an administrator actually manages.
    let connector = async_native_tls::TlsConnector::new();

    connector
        .connect(host, stream)
        .await
        .map(|stream| Stream::Tls(Box::new(stream)))
}

/// Turns a server's refusal into something the user can act on.
///
/// This is the heart of the module. The mapping is by provider *and* by response text,
/// because the same "authentication failed" means a different fix on each provider — and
/// docs/05 §3 in particular calls out two Microsoft failures that are indistinguishable
/// from a wrong password unless you read the code.
fn diagnose(provider: Provider, response: &str, is_smtp: bool) -> String {
    let text = response.to_ascii_lowercase();

    // ---- Microsoft, docs/05 §3 --------------------------------------------------------
    // 5.7.139 is specifically "SMTP AUTH is disabled for this mailbox" — an admin setting,
    // per-mailbox, and the single most common Microsoft 365 failure for a third-party
    // client. Telling the user their password is wrong here is actively misleading.
    if text.contains("5.7.139") || text.contains("smtpclientauthentication") {
        return "Your organisation has turned off SMTP authentication for this mailbox. An \
                administrator has to enable SMTP AUTH for it in the Microsoft 365 admin \
                centre — your password is not the problem."
            .into();
    }

    if text.contains("imap is disabled") || text.contains("tenant") && text.contains("disabled") {
        return "Your organisation has turned off IMAP access for third-party apps. An \
                administrator has to enable it before Halcyon can connect."
            .into();
    }

    if text.contains("basic authentication") || text.contains("basic auth") {
        return "Microsoft has switched this account off basic authentication. Sign in with \
                the Microsoft option instead of a password."
            .into();
    }

    // ---- Google, docs/05 §2 ------------------------------------------------------------
    if text.contains("application-specific password") {
        return "This Google account has two-step verification turned on, so a password will \
                not work. Sign in with the Google option instead."
            .into();
    }

    // ---- Apple, docs/05 §4 -------------------------------------------------------------
    if provider == Provider::ICloud
        && (text.contains("authenticationfailed") || text.contains("invalid credentials"))
    {
        return "iCloud rejected the password. It needs an app-specific password generated at \
                appleid.apple.com — your Apple ID password will not work, and the app-specific \
                password is only shown once, so it may need generating again."
            .into();
    }

    if provider == Provider::Yahoo
        && (text.contains("authenticationfailed") || text.contains("invalid credentials"))
    {
        return "Yahoo rejected the password. Yahoo needs an app password generated under \
                Account Security, not your normal Yahoo password."
            .into();
    }

    // ---- Rate limiting, docs/05 §5 -----------------------------------------------------
    if text.contains("too many") || text.contains("rate limit") || text.contains("try again later")
    {
        return "The server is refusing new connections for now, usually because too many were \
                opened too quickly. Waiting a few minutes and trying again normally clears it."
            .into();
    }

    // ---- Generic ------------------------------------------------------------------------
    if text.contains("authenticationfailed")
        || text.contains("invalid credentials")
        || text.contains("login failed")
        || text.contains("535")
    {
        return "The server rejected the username or password. Check the address is spelled \
                correctly and that the password is the one for this mail account."
            .into();
    }

    if is_smtp {
        "The outgoing server refused the sign-in. The username and password for sending are \
         usually the same as for receiving; check with your provider if they are not."
            .into()
    } else {
        "The incoming server refused the sign-in.".into()
    }
}

fn connect_remedy(host: &str, port: u16) -> String {
    format!(
        "Nothing answered at {host} on port {port}. Check the server name is spelled correctly, \
         and that a firewall or VPN is not blocking the connection."
    )
}

fn quote(value: &str) -> String {
    // IMAP quoted strings escape only the backslash and the double quote. Without this an
    // address or password containing either would end the string early and produce a
    // protocol error that reads like a rejected password.
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn xoauth2(email: &str, token: &Secret) -> String {
    // The SASL XOAUTH2 initial client response, per Google's and Microsoft's identical
    // definitions: user=<email>^Aauth=Bearer <token>^A^A, base64-encoded.
    let raw = format!("user={}\x01auth=Bearer {}\x01\x01", email, token.expose());
    base64::engine::general_purpose::STANDARD.encode(raw)
}

/// The credential to test with, already loaded.
pub enum Attempt {
    Password(Secret),
    OAuth { access_token: Secret },
}

async fn check_imap(
    server: &ServerSettings,
    email: &str,
    provider: Provider,
    attempt: &Attempt,
) -> Vec<CheckStep> {
    let mut report = Report::new();
    let started = Instant::now();

    // ---- connect -----------------------------------------------------------------------
    let tcp = match tokio::time::timeout(
        STEP_TIMEOUT,
        TcpStream::connect((server.host.as_str(), server.port)),
    )
    .await
    {
        Ok(Ok(stream)) => {
            report.passed(
                "Connect",
                format!("Reached {}:{}.", server.host, server.port),
                started,
            );
            stream
        }
        Ok(Err(error)) => {
            report.failed(
                "Connect",
                format!("Could not reach {}:{}.", server.host, server.port),
                Some(connect_remedy(&server.host, server.port)),
                Some(error.to_string()),
                started,
            );
            for step in ["Secure the connection", "Sign in", "Open Inbox"] {
                report.skip(step, "Not attempted — the server could not be reached.");
            }
            return report.steps;
        }
        Err(_) => {
            report.failed(
                "Connect",
                format!("{}:{} did not answer.", server.host, server.port),
                Some(connect_remedy(&server.host, server.port)),
                None,
                started,
            );
            for step in ["Secure the connection", "Sign in", "Open Inbox"] {
                report.skip(step, "Not attempted — the server did not answer.");
            }
            return report.steps;
        }
    };

    // ---- TLS ---------------------------------------------------------------------------
    let started = Instant::now();
    let mut stream = if server.security == Security::Tls {
        match tls_upgrade(tcp, &server.host).await {
            Ok(stream) => {
                report.passed(
                    "Secure the connection",
                    "The server presented a valid certificate.",
                    started,
                );
                stream
            }
            Err(error) => {
                report.failed(
                    "Secure the connection",
                    "The encrypted connection could not be established.",
                    Some(
                        "The server's security certificate was not accepted. This can mean the \
                         server name is wrong, the certificate has expired, or something on the \
                         network is intercepting the connection. Halcyon will not connect \
                         without a valid certificate."
                            .into(),
                    ),
                    Some(error.to_string()),
                    started,
                );
                report.skip("Sign in", "Not attempted — the connection is not secure.");
                report.skip(
                    "Open Inbox",
                    "Not attempted — the connection is not secure.",
                );
                return report.steps;
            }
        }
    } else {
        Stream::Plain(tcp)
    };

    // ---- greeting ----------------------------------------------------------------------
    let complete = |text: &str| text.ends_with("\r\n");
    if let Err(error) = stream.read_response(complete).await {
        report.failed(
            "Sign in",
            "The server did not send a greeting.",
            Some(
                "The port answered, but nothing on it behaves like an IMAP server. Check the \
                  server name and port."
                    .into(),
            ),
            Some(error),
            Instant::now(),
        );
        report.skip("Open Inbox", "Not attempted — no IMAP greeting.");
        return report.steps;
    }

    // STARTTLS on 143 would be upgraded here; the settings model reaches this branch only
    // for a manually configured plaintext IMAP port, which docs/05 §6 does not permit
    // against a public host. Rejected explicitly rather than proceeding in the clear.
    if server.security == Security::StartTls && matches!(stream, Stream::Plain(_)) {
        let started = Instant::now();
        if let Err(error) = stream.write_line("a1 STARTTLS").await {
            report.failed(
                "Secure the connection",
                "STARTTLS could not be requested.",
                Some(connect_remedy(&server.host, server.port)),
                Some(error.to_string()),
                started,
            );
            report.skip("Sign in", "Not attempted — the connection is not secure.");
            report.skip(
                "Open Inbox",
                "Not attempted — the connection is not secure.",
            );
            return report.steps;
        }

        let response = stream.read_response(|text| text.contains("a1 ")).await;
        let upgraded = matches!(&response, Ok(text) if text.contains("a1 OK"));

        if !upgraded {
            report.failed(
                "Secure the connection",
                "The server refused to start an encrypted session.",
                Some(
                    "Halcyon only connects over an encrypted connection. This server declined \
                     to start one on this port — port 993 is usually the right choice."
                        .into(),
                ),
                response.ok(),
                started,
            );
            report.skip("Sign in", "Not attempted — the connection is not secure.");
            report.skip(
                "Open Inbox",
                "Not attempted — the connection is not secure.",
            );
            return report.steps;
        }

        // The upgraded stream is not carried forward: a correct STARTTLS flow rebuilds the
        // connection here. Reaching this branch means a manually entered plaintext port,
        // which is out of scope for Phase 4's supported providers — reported rather than
        // silently continued unencrypted.
        report.failed(
            "Secure the connection",
            "STARTTLS on IMAP is not supported yet.",
            Some(
                "Use port 993 with TLS. Halcyon does not yet support upgrading a plaintext \
                 IMAP connection, and will not fall back to an unencrypted one."
                    .into(),
            ),
            None,
            started,
        );
        report.skip("Sign in", "Not attempted.");
        report.skip("Open Inbox", "Not attempted.");
        return report.steps;
    }

    // ---- authenticate -------------------------------------------------------------------
    let started = Instant::now();

    let command = match attempt {
        Attempt::Password(secret) => {
            format!("a2 LOGIN {} {}", quote(email), quote(secret.expose()))
        }
        Attempt::OAuth { access_token } => {
            format!("a2 AUTHENTICATE XOAUTH2 {}", xoauth2(email, access_token))
        }
    };

    if let Err(error) = stream.write_line(&command).await {
        report.failed(
            "Sign in",
            "The sign-in could not be sent.",
            Some(connect_remedy(&server.host, server.port)),
            Some(error.to_string()),
            started,
        );
        report.skip("Open Inbox", "Not attempted — sign-in failed.");
        return report.steps;
    }

    let response = match stream
        .read_response(|text| text.contains("\r\na2 ") || text.starts_with("a2 "))
        .await
    {
        Ok(text) => text,
        Err(error) => {
            report.failed(
                "Sign in",
                "The server stopped responding during sign-in.",
                Some(connect_remedy(&server.host, server.port)),
                Some(error),
                started,
            );
            report.skip("Open Inbox", "Not attempted — sign-in failed.");
            return report.steps;
        }
    };

    if !response.contains("a2 OK") {
        // The raw response goes into `server_said` and never into a log — it can echo the
        // username, and a failed LOGIN response has been known to quote the command.
        report.failed(
            "Sign in",
            "The server rejected the sign-in.",
            Some(diagnose(provider, &response, false)),
            Some(redact_command_echo(&response)),
            started,
        );
        report.skip("Open Inbox", "Not attempted — sign-in failed.");
        return report.steps;
    }

    report.passed("Sign in", "Signed in successfully.", started);

    // ---- open the inbox -----------------------------------------------------------------
    // Authenticating proves the credential; selecting INBOX proves the account can actually
    // read mail. Microsoft in particular authenticates fine and then refuses the mailbox
    // when the tenant has disabled IMAP.
    let started = Instant::now();
    let _ = stream.write_line("a3 SELECT INBOX").await;

    match stream.read_response(|text| text.contains("\r\na3 ")).await {
        Ok(text) if text.contains("a3 OK") => {
            let count = text
                .lines()
                .find_map(|line| line.strip_suffix(" EXISTS")?.strip_prefix("* "))
                .unwrap_or("some");

            report.passed(
                "Open Inbox",
                format!("Inbox opened — {count} messages."),
                started,
            );
        }
        Ok(text) => {
            report.failed(
                "Open Inbox",
                "Signed in, but the Inbox could not be opened.",
                Some(diagnose(provider, &text, false)),
                Some(redact_command_echo(&text)),
                started,
            );
        }
        Err(error) => {
            report.failed(
                "Open Inbox",
                "The server stopped responding after sign-in.",
                Some(connect_remedy(&server.host, server.port)),
                Some(error),
                started,
            );
        }
    }

    let _ = stream.write_line("a4 LOGOUT").await;
    report.steps
}

async fn check_smtp(
    server: &ServerSettings,
    email: &str,
    provider: Provider,
    attempt: &Attempt,
) -> Vec<CheckStep> {
    let mut report = Report::new();
    let started = Instant::now();

    let tcp = match tokio::time::timeout(
        STEP_TIMEOUT,
        TcpStream::connect((server.host.as_str(), server.port)),
    )
    .await
    {
        Ok(Ok(stream)) => {
            report.passed(
                "Connect",
                format!("Reached {}:{}.", server.host, server.port),
                started,
            );
            stream
        }
        _ => {
            report.failed(
                "Connect",
                format!("Could not reach {}:{}.", server.host, server.port),
                Some(connect_remedy(&server.host, server.port)),
                None,
                started,
            );
            report.skip("Secure the connection", "Not attempted.");
            report.skip("Sign in", "Not attempted.");
            return report.steps;
        }
    };

    // SMTP responses end with a line whose fourth character is a space rather than a hyphen.
    let complete = |text: &str| {
        text.lines()
            .last()
            .map(|line| line.len() >= 4 && line.as_bytes()[3] == b' ')
            .unwrap_or(false)
            && text.ends_with("\r\n")
    };

    let started = Instant::now();
    let mut stream = if server.security == Security::Tls {
        match tls_upgrade(tcp, &server.host).await {
            Ok(stream) => {
                report.passed(
                    "Secure the connection",
                    "The server presented a valid certificate.",
                    started,
                );
                stream
            }
            Err(error) => {
                report.failed(
                    "Secure the connection",
                    "The encrypted connection could not be established.",
                    Some(
                        "The outgoing server's certificate was not accepted. Check the server \
                         name, and whether something on the network is intercepting the \
                         connection."
                            .into(),
                    ),
                    Some(error.to_string()),
                    started,
                );
                report.skip("Sign in", "Not attempted — the connection is not secure.");
                return report.steps;
            }
        }
    } else {
        Stream::Plain(tcp)
    };

    if stream.read_response(complete).await.is_err() {
        report.failed(
            "Sign in",
            "The server did not send a greeting.",
            Some("The port answered, but nothing on it behaves like an SMTP server.".into()),
            None,
            Instant::now(),
        );
        return report.steps;
    }

    let ehlo = format!("EHLO {}", client_hostname());
    let _ = stream.write_line(&ehlo).await;
    let capabilities = stream.read_response(complete).await.unwrap_or_default();

    // ---- STARTTLS ------------------------------------------------------------------------
    if server.security == Security::StartTls {
        let started = Instant::now();

        if !capabilities.to_ascii_uppercase().contains("STARTTLS") {
            report.failed(
                "Secure the connection",
                "The server does not offer STARTTLS on this port.",
                Some(
                    "Halcyon will not send mail over an unencrypted connection. Port 587 with \
                     STARTTLS, or port 465 with TLS, is usually the right choice."
                        .into(),
                ),
                Some(capabilities),
                started,
            );
            report.skip("Sign in", "Not attempted — the connection is not secure.");
            return report.steps;
        }

        let _ = stream.write_line("STARTTLS").await;
        let response = stream.read_response(complete).await.unwrap_or_default();

        if !response.starts_with("220") {
            report.failed(
                "Secure the connection",
                "The server refused to start an encrypted session.",
                Some(
                    "The server advertised STARTTLS but declined it. This is usually a server \
                     misconfiguration; try port 465 with TLS instead."
                        .into(),
                ),
                Some(response),
                started,
            );
            report.skip("Sign in", "Not attempted — the connection is not secure.");
            return report.steps;
        }

        let plain = match stream {
            Stream::Plain(tcp) => tcp,
            // Unreachable: this branch only runs when the stream was left plain above.
            Stream::Tls(_) => {
                report.failed(
                    "Secure the connection",
                    "The connection was already encrypted.",
                    Some("Try port 465 with TLS instead of STARTTLS.".into()),
                    None,
                    started,
                );
                report.skip("Sign in", "Not attempted.");
                return report.steps;
            }
        };

        stream = match tls_upgrade(plain, &server.host).await {
            Ok(stream) => {
                report.passed(
                    "Secure the connection",
                    "Upgraded to an encrypted connection with a valid certificate.",
                    started,
                );
                stream
            }
            Err(error) => {
                report.failed(
                    "Secure the connection",
                    "The encrypted connection could not be established.",
                    Some(
                        "The outgoing server's certificate was not accepted after STARTTLS. \
                         Check the server name, and whether something on the network is \
                         intercepting the connection."
                            .into(),
                    ),
                    Some(error.to_string()),
                    started,
                );
                report.skip("Sign in", "Not attempted — the connection is not secure.");
                return report.steps;
            }
        };

        // RFC 3207 requires a second EHLO after the upgrade; the capability list before
        // TLS is not to be trusted, and AUTH is usually only advertised afterwards.
        let _ = stream.write_line(&ehlo).await;
        let _ = stream.read_response(complete).await;
    }

    // ---- authenticate ---------------------------------------------------------------------
    let started = Instant::now();

    match attempt {
        Attempt::OAuth { access_token } => {
            let _ = stream
                .write_line(&format!("AUTH XOAUTH2 {}", xoauth2(email, access_token)))
                .await;
        }
        Attempt::Password(secret) => {
            // AUTH PLAIN: base64 of NUL user NUL password. Sent on one line rather than as
            // a challenge exchange, which every server here supports.
            let raw = format!("\0{}\0{}", email, secret.expose());
            let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
            let _ = stream.write_line(&format!("AUTH PLAIN {encoded}")).await;
        }
    }

    let response = stream.read_response(complete).await.unwrap_or_default();

    if response.starts_with("235") {
        report.passed("Sign in", "Signed in to the outgoing server.", started);
    } else {
        report.failed(
            "Sign in",
            "The outgoing server rejected the sign-in.",
            Some(diagnose(provider, &response, true)),
            Some(response),
            started,
        );
    }

    let _ = stream.write_line("QUIT").await;
    report.steps
}

/// The name given in EHLO.
///
/// Deliberately not the machine's real hostname: it would be sent to every mail server the
/// user connects to, and a laptop named after its owner is a small privacy leak for no
/// benefit. Servers do not check it.
fn client_hostname() -> &'static str {
    "[127.0.0.1]"
}

/// Strips anything that looks like a credential out of a server response before it is shown.
///
/// Some servers echo the failed command back. A LOGIN echo would put the password on screen
/// and, worse, into a screenshot in a support thread.
fn redact_command_echo(response: &str) -> String {
    response
        .lines()
        .map(|line| {
            let upper = line.to_ascii_uppercase();
            if upper.contains("LOGIN ") || upper.contains("AUTHENTICATE") || upper.contains("AUTH ")
            {
                "(command echoed by the server, hidden because it contains the password)"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Runs the whole test. Both halves run concurrently — they are independent, and a user
/// waiting on a spinner should not wait for the sum of two ten-second timeouts.
pub async fn run(
    email: &str,
    provider: Provider,
    imap: &ServerSettings,
    smtp: &ServerSettings,
    attempt: Attempt,
) -> DiagnosticReport {
    let (imap_steps, smtp_steps) = futures::future::join(
        check_imap(imap, email, provider, &attempt),
        check_smtp(smtp, email, provider, &attempt),
    )
    .await;

    DiagnosticReport::build(imap_steps, smtp_steps)
}

/// Which credential kind this account's auth needs.
pub fn attempt_kind(auth: AuthKind) -> super::credentials::Kind {
    match auth {
        AuthKind::OAuth2 => super::credentials::Kind::AccessToken,
        AuthKind::Password => super::credentials::Kind::Password,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsoft_smtp_auth_being_disabled_is_not_reported_as_a_wrong_password() {
        // docs/05 §3. 5.7.139 is an administrator setting on the mailbox. Every other client
        // says "authentication failed" here, and the user changes their password over and
        // over. This single mapping is most of the reason the module exists.
        let response = "535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is \
                        disabled for the Tenant.";

        let remedy = diagnose(Provider::Microsoft, response, true);

        assert!(remedy.contains("administrator"), "{remedy}");
        assert!(remedy.contains("SMTP"), "{remedy}");
        assert!(
            remedy.contains("password is not the problem"),
            "it must say what is *not* wrong: {remedy}"
        );
    }

    #[test]
    fn icloud_is_told_it_needs_an_app_specific_password() {
        let remedy = diagnose(
            Provider::ICloud,
            "a2 NO [AUTHENTICATIONFAILED] Authentication failed.",
            false,
        );

        assert!(remedy.contains("app-specific"), "{remedy}");
        assert!(remedy.contains("appleid.apple.com"), "{remedy}");
    }

    #[test]
    fn yahoo_is_told_about_app_passwords_and_icloud_is_not_told_about_yahoo() {
        let yahoo = diagnose(Provider::Yahoo, "NO [AUTHENTICATIONFAILED] bad", false);
        assert!(yahoo.contains("app password"), "{yahoo}");
        assert!(yahoo.contains("Account Security"), "{yahoo}");

        // The provider has to select the remedy, or every user gets every provider's advice.
        let icloud = diagnose(Provider::ICloud, "NO [AUTHENTICATIONFAILED] bad", false);
        assert!(!icloud.contains("Yahoo"), "{icloud}");
    }

    #[test]
    fn a_google_account_with_two_step_is_sent_to_oauth_not_told_to_retype_its_password() {
        let remedy = diagnose(
            Provider::Google,
            "NO [ALERT] Application-specific password required",
            false,
        );

        assert!(remedy.contains("two-step"), "{remedy}");
        assert!(remedy.contains("Google option"), "{remedy}");
    }

    #[test]
    fn throttling_is_distinguished_from_a_bad_password() {
        // docs/05 §5. Telling a rate-limited user to check their password sends them to
        // change a password that was never wrong.
        let remedy = diagnose(
            Provider::Yahoo,
            "NO [UNAVAILABLE] Too many connections",
            false,
        );

        assert!(remedy.contains("few minutes"), "{remedy}");
        assert!(!remedy.contains("password"), "{remedy}");
    }

    #[test]
    fn an_unrecognised_refusal_still_produces_a_usable_sentence() {
        // The fallback matters more than the specific mappings: an unknown server must not
        // produce an empty remedy or a raw protocol string.
        let remedy = diagnose(Provider::Other, "NO something went wrong", false);

        assert!(!remedy.is_empty());
        assert!(remedy.ends_with('.'), "{remedy}");
    }

    #[test]
    fn a_password_echoed_back_by_the_server_is_never_shown() {
        // Standing rule 12 reaches the diagnostic pane too. A support screenshot of this
        // report must not contain the password.
        let response = "a2 BAD Invalid command: a2 LOGIN \"ada@example.test\" \"hunter2\"";
        let shown = redact_command_echo(response);

        assert!(!shown.contains("hunter2"));
        assert!(shown.contains("hidden because it contains the password"));
    }

    #[test]
    fn imap_quoting_escapes_what_would_otherwise_end_the_string() {
        // A password containing a quote would otherwise produce a protocol error that reads
        // exactly like a rejected password — an hour of a user's life.
        assert_eq!(quote(r#"pa"ss"#), r#""pa\"ss""#);
        assert_eq!(quote(r"pa\ss"), r#""pa\\ss""#);
        assert_eq!(quote("plain"), "\"plain\"");
    }

    #[test]
    fn the_xoauth2_response_has_the_exact_shape_both_providers_require() {
        // A malformed initial response is rejected with a message that says nothing about
        // the format being wrong, so this is asserted byte for byte.
        let encoded = xoauth2("ada@example.test", &Secret::new("token-123"));
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("valid base64");

        assert_eq!(
            String::from_utf8(decoded).expect("utf-8"),
            "user=ada@example.test\x01auth=Bearer token-123\x01\x01"
        );
    }

    #[test]
    fn the_ehlo_name_is_not_the_machines_hostname() {
        // It is sent to every server the user connects to. A laptop named after its owner
        // would leak that for no benefit.
        let name = client_hostname();

        assert_eq!(name, "[127.0.0.1]");
        assert!(!name.contains(char::is_alphabetic));
    }

    #[test]
    fn the_summary_is_the_first_failures_remedy() {
        // The sheet shows one line at the top. Showing the *last* step's message would show
        // a cascade of skips instead of the thing that actually broke.
        let steps = vec![
            CheckStep {
                name: "Connect".into(),
                status: StepStatus::Passed,
                detail: "fine".into(),
                remedy: None,
                server_said: None,
                elapsed_ms: 1,
            },
            CheckStep {
                name: "Sign in".into(),
                status: StepStatus::Failed,
                detail: "rejected".into(),
                remedy: Some("Do the thing.".into()),
                server_said: None,
                elapsed_ms: 2,
            },
        ];

        let report = DiagnosticReport::build(steps, Vec::new());

        assert!(!report.ok);
        assert_eq!(report.summary, "Do the thing.");
    }

    #[test]
    fn an_all_passing_report_says_so() {
        let passed = |name: &str| CheckStep {
            name: name.into(),
            status: StepStatus::Passed,
            detail: "fine".into(),
            remedy: None,
            server_said: None,
            elapsed_ms: 1,
        };

        let report = DiagnosticReport::build(vec![passed("Connect")], vec![passed("Connect")]);

        assert!(report.ok);
        assert!(report.summary.contains("successfully"));
    }

    #[tokio::test]
    async fn a_server_that_is_not_there_produces_a_remedy_and_skips_the_rest() {
        // Port 1 on the loopback interface: nothing listens there, and the connection is
        // refused immediately rather than timing out, so this stays fast.
        let server = ServerSettings {
            host: "127.0.0.1".into(),
            port: 1,
            security: Security::Tls,
        };

        let steps = check_imap(
            &server,
            "ada@example.test",
            Provider::Other,
            &Attempt::Password(Secret::new("x")),
        )
        .await;

        assert_eq!(steps[0].name, "Connect");
        assert_eq!(steps[0].status, StepStatus::Failed);
        assert!(steps[0]
            .remedy
            .as_deref()
            .unwrap_or("")
            .contains("firewall"));

        // The remaining steps are reported as skipped rather than omitted — a report that
        // stops halfway looks like the app crashed.
        assert_eq!(steps.len(), 4);
        assert!(steps[1..].iter().all(|s| s.status == StepStatus::Skipped));
    }

    #[tokio::test]
    async fn a_port_that_answers_but_is_not_a_mail_server_is_reported_as_such() {
        // A listener that accepts and says nothing. Without a greeting timeout this would
        // hang; the point of the test is that it fails with a sentence instead.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();

        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                // Held open, silent, then dropped — the shape of a non-mail service.
                drop(stream);
            }
        });

        let server = ServerSettings {
            host: "127.0.0.1".into(),
            port,
            security: Security::StartTls, // avoids a TLS handshake against a plain socket
        };

        let steps = check_imap(
            &server,
            "ada@example.test",
            Provider::Other,
            &Attempt::Password(Secret::new("x")),
        )
        .await;

        assert_eq!(
            steps[0].status,
            StepStatus::Passed,
            "the socket does answer"
        );
        assert!(
            steps.iter().any(|s| s.status == StepStatus::Failed),
            "but the test must not report success"
        );
    }
}
