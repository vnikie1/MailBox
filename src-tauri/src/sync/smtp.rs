//! Handing a message to the outgoing server. docs/06 Phase 7.
//!
//! Thin on purpose. The interesting decisions are in `outbox` — what to send, when, and what to
//! do when it fails — and this is the part that opens a socket and reports what the server
//! said. Keeping it thin is what lets the outbox's state machine be tested without a network.
//!
//! Two things here are policy rather than plumbing:
//!
//! * **Encryption is not optional.** The same rule as IMAP (docs/05 §6): STARTTLS on 587 or
//!   implicit TLS on 465, certificates validated against the system trust store, and no
//!   user-visible bypass. Submitting mail in the clear hands the message and the credential to
//!   anyone on the path.
//! * **A permanent rejection is not retried.** A 5xx means the server has decided; trying again
//!   produces the same answer and, on the way, looks like a client that will not take no for an
//!   answer. 4xx is temporary by definition and is worth another attempt.

use lettre::address::Envelope as SmtpEnvelope;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::{Address as LettreAddress, AsyncTransport, Tokio1Executor};

use crate::accounts::credentials::Secret;
use crate::accounts::provider::{Security, ServerSettings};

use super::session::Credential;

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("{host}:{port} is not an encrypted submission port")]
    Insecure { host: String, port: u16 },

    #[error("could not build a connection to {host}: {detail}")]
    Transport { host: String, detail: String },

    #[error("{host} refused the message: {detail}")]
    Refused { host: String, detail: String },

    #[error("{host} could not take the message right now: {detail}")]
    Temporary { host: String, detail: String },

    #[error("the message could not be read back for sending: {0}")]
    Unreadable(#[from] std::io::Error),

    #[error("the message has no usable envelope: {detail}")]
    Envelope { detail: String },
}

impl SendError {
    /// Whether another attempt could plausibly succeed.
    ///
    /// A 5xx is the server's final answer. Retrying it wastes the user's time and, repeated
    /// often enough, is the behaviour that gets a sender rate-limited.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            SendError::Temporary { .. } | SendError::Transport { .. }
        )
    }
}

/// Classifies what the server said.
///
/// SMTP reply codes are the one part of the protocol that is genuinely unambiguous: the first
/// digit says everything. 4 is "not now", 5 is "no".
fn classify(host: &str, error: &lettre::transport::smtp::Error) -> SendError {
    let detail = error.to_string();

    if let Some(code) = error.status() {
        return match code.severity {
            lettre::transport::smtp::response::Severity::PermanentNegativeCompletion => {
                SendError::Refused {
                    host: host.to_string(),
                    detail,
                }
            }
            _ => SendError::Temporary {
                host: host.to_string(),
                detail,
            },
        };
    }

    // No status: the failure was below the protocol — a dropped connection, a TLS problem, a
    // name that would not resolve. All worth another attempt.
    SendError::Transport {
        host: host.to_string(),
        detail,
    }
}

/// Which SASL mechanisms to offer, and the secret to offer them with.
///
/// ## Why an OAuth account is offered XOAUTH2 and nothing else
///
/// `lettre` picks the first mechanism in this list that the server also advertises. Leaving
/// PLAIN in the list for an OAuth account would let it win, and PLAIN would then send the
/// **access token in the password field**. Gmail rejects that, so it would look merely broken --
/// but the token would already have been transmitted as a password to a server that has every
/// reason to log a failed authentication. A bearer token is not a password and must not be sent
/// where one is expected.
///
/// The reverse is just as deliberate: a password account is never offered XOAUTH2, because a
/// password in a `auth=Bearer` field is not a credential the server can check.
fn mechanisms_for(credential: &Credential) -> (&Secret, Vec<Mechanism>) {
    match credential {
        // PLAIN and LOGIN only over an already-encrypted channel, which the `Tls` setting in
        // `transport` guarantees.
        Credential::Password(secret) => (secret, vec![Mechanism::Plain, Mechanism::Login]),
        Credential::OAuth(token) => (token, vec![Mechanism::Xoauth2]),
    }
}

/// Builds a transport for one account's submission server.
fn transport(
    server: &ServerSettings,
    email: &str,
    credential: &Credential,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, SendError> {
    // Port 25 is relay, not submission, and is blocked by most networks anyway. Anything
    // unencrypted is refused outright rather than downgraded — docs/05 §6.
    if server.port == 25 {
        return Err(SendError::Insecure {
            host: server.host.clone(),
            port: server.port,
        });
    }

    let tls = TlsParameters::new(server.host.clone()).map_err(|error| SendError::Transport {
        host: server.host.clone(),
        detail: error.to_string(),
    })?;

    let (secret, mechanisms) = mechanisms_for(credential);

    // For XOAUTH2 the second field is the access token, not a password: `lettre` renders it as
    // `user=<email>auth=Bearer <token>`, which is the same SASL string the IMAP
    // side builds by hand in `session.rs`.
    //
    // `builder_dangerous` names the *unencrypted* default it starts from; the encryption is
    // applied below and is not optional.
    let builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&server.host)
        .port(server.port)
        .credentials(Credentials::new(
            email.to_string(),
            secret.expose().to_string(),
        ))
        .authentication(mechanisms)
        .tls(match server.security {
            Security::Tls => Tls::Wrapper(tls),
            Security::StartTls => Tls::Required(tls),
        });

    Ok(builder.build())
}

/// The delivery envelope for a message.
///
/// Built from the addresses rather than recovered from the transmitted bytes, and that is the
/// mechanism by which `Bcc` works: the blind recipients are in `recipients` here, so the server
/// delivers to them, and they are absent from the headers `outgoing::build` wrote, so nobody
/// receiving the message can see them. The two lists are deliberately different, and separating
/// them is the only way to get the behaviour right.
pub fn envelope_for(from: &str, recipients: &[String]) -> Result<SmtpEnvelope, SendError> {
    let sender = from
        .trim()
        .parse::<LettreAddress>()
        .map_err(|error| SendError::Envelope {
            detail: format!("From ({}): {error}", from.trim()),
        })?;

    let mut to = Vec::with_capacity(recipients.len());
    for address in recipients {
        to.push(
            address
                .trim()
                .parse::<LettreAddress>()
                .map_err(|error| SendError::Envelope {
                    detail: format!("{}: {error}", address.trim()),
                })?,
        );
    }

    if to.is_empty() {
        return Err(SendError::Envelope {
            detail: "no recipients".into(),
        });
    }

    SmtpEnvelope::new(Some(sender), to).map_err(|error| SendError::Envelope {
        detail: error.to_string(),
    })
}

/// Sends one already-built message.
pub async fn send(
    server: &ServerSettings,
    email: &str,
    credential: &Credential,
    envelope: &SmtpEnvelope,
    raw: &[u8],
) -> Result<(), SendError> {
    let transport = transport(server, email, credential)?;

    transport
        .send_raw(envelope, raw)
        .await
        .map_err(|error| classify(&server.host, &error))?;

    Ok(())
}

/// Whether an address is one this transport could deliver to.
///
/// Checked before a message is queued rather than after a connection is opened, so a typo is
/// caught while the compose window is still on screen and the text still in front of the user.
pub fn is_deliverable(address: &str) -> bool {
    address.trim().parse::<LettreAddress>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(port: u16, security: Security) -> ServerSettings {
        ServerSettings {
            host: "smtp.example.test".into(),
            port,
            security,
        }
    }

    #[test]
    fn an_oauth_account_is_never_offered_a_password_mechanism() {
        // The one that matters. `lettre` picks the first mechanism the server also advertises,
        // so leaving PLAIN in the list would let it win -- and PLAIN puts the second credential
        // field in the password slot. That field holds an OAuth **access token**.
        //
        // Gmail would reject it, so the visible symptom would be "sending is broken" rather than
        // anything alarming. The real cost is silent: the token would already have been sent as a
        // password to a server with every reason to log a failed authentication.
        let credential = Credential::OAuth(Secret::new("ya29.a-real-looking-access-token"));
        let (secret, mechanisms) = mechanisms_for(&credential);

        assert_eq!(
            mechanisms,
            vec![Mechanism::Xoauth2],
            "an OAuth account must offer XOAUTH2 and nothing else"
        );
        assert!(!mechanisms.contains(&Mechanism::Plain));
        assert!(!mechanisms.contains(&Mechanism::Login));
        assert_eq!(secret.expose(), "ya29.a-real-looking-access-token");
    }

    #[test]
    fn a_password_account_is_never_offered_xoauth2() {
        // The mirror image, and not symmetry for its own sake: a password rendered into an
        // `auth=Bearer` field is not a credential the server can check, so the send would fail
        // with an authentication error that says nothing about the cause.
        let credential = Credential::Password(Secret::new("hunter2"));
        let (secret, mechanisms) = mechanisms_for(&credential);

        assert!(!mechanisms.contains(&Mechanism::Xoauth2));
        assert!(mechanisms.contains(&Mechanism::Plain));
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn the_relay_port_is_refused_rather_than_used() {
        // Port 25 is relay, not submission. Submitting there is unauthenticated by convention,
        // blocked by most networks, and in the clear.
        let error = transport(
            &server(25, Security::StartTls),
            "me@x.test",
            &Credential::Password(Secret::new("p")),
        )
        .expect_err("should refuse");

        assert!(matches!(error, SendError::Insecure { port: 25, .. }));
    }

    #[test]
    fn both_encrypted_submission_ports_build_a_transport() {
        // 587 with STARTTLS and 465 with implicit TLS are the two shapes every provider offers.
        assert!(transport(
            &server(587, Security::StartTls),
            "me@x.test",
            &Credential::Password(Secret::new("p"))
        )
        .is_ok());
        assert!(transport(
            &server(465, Security::Tls),
            "me@x.test",
            &Credential::Password(Secret::new("p"))
        )
        .is_ok());
    }

    #[test]
    fn a_permanent_refusal_is_not_retried_and_a_temporary_one_is() {
        // The distinction the outbox branches on. Retrying a 5xx produces the same answer and
        // is the behaviour that gets a sender rate-limited.
        let refused = SendError::Refused {
            host: "smtp.example.test".into(),
            detail: "550 no such user".into(),
        };
        assert!(!refused.is_retryable());

        let temporary = SendError::Temporary {
            host: "smtp.example.test".into(),
            detail: "451 try again later".into(),
        };
        assert!(temporary.is_retryable());

        let transport = SendError::Transport {
            host: "smtp.example.test".into(),
            detail: "connection reset".into(),
        };
        assert!(transport.is_retryable());

        // Not retryable: the port will still be the wrong port next time.
        let insecure = SendError::Insecure {
            host: "smtp.example.test".into(),
            port: 25,
        };
        assert!(!insecure.is_retryable());
    }

    #[test]
    fn no_error_carries_the_password() {
        // Standing rule 12 reaches here too. These strings are logged on every failure and end
        // up in the outbox's `last_error` column, which the UI shows.
        for error in [
            SendError::Refused {
                host: "smtp.example.test".into(),
                detail: "535 authentication failed".into(),
            },
            SendError::Insecure {
                host: "smtp.example.test".into(),
                port: 25,
            },
        ] {
            let rendered = error.to_string();
            assert!(rendered.contains("smtp.example.test"));
            assert!(!rendered.contains("hunter2"));
        }
    }

    #[test]
    fn a_typo_is_caught_before_a_connection_is_opened() {
        assert!(is_deliverable("ada@example.test"));
        assert!(is_deliverable("  ada@example.test  "));

        assert!(!is_deliverable("ada"));
        assert!(!is_deliverable("ada@"));
        assert!(!is_deliverable("not an address"));
        assert!(!is_deliverable(""));
    }

    #[test]
    fn the_envelope_carries_blind_recipients_that_the_headers_do_not() {
        // This is the mechanism by which Bcc works, and the reason the envelope is built from
        // the addresses rather than recovered from the transmitted bytes: the server is told to
        // deliver to the blind recipient, and nothing in the message says so.
        let envelope = envelope_for(
            "me@halcyon.test",
            &[
                "ada@example.test".to_string(),
                "secret@example.test".to_string(),
            ],
        )
        .expect("envelope");

        let recipients: Vec<String> = envelope.to().iter().map(ToString::to_string).collect();

        assert_eq!(recipients, ["ada@example.test", "secret@example.test"]);
        assert_eq!(
            envelope.from().map(ToString::to_string),
            Some("me@halcyon.test".to_string())
        );
    }

    #[test]
    fn an_envelope_with_nobody_to_deliver_to_is_refused() {
        // Better than opening a connection to be told the same thing by the server.
        assert!(matches!(
            envelope_for("me@halcyon.test", &[]),
            Err(SendError::Envelope { .. })
        ));
    }

    #[test]
    fn a_bad_address_in_the_envelope_names_itself() {
        // "Invalid recipient" alone means reading a token field to find which one.
        match envelope_for("me@halcyon.test", &["not an address".to_string()]) {
            Err(SendError::Envelope { detail }) => {
                assert!(detail.contains("not an address"), "{detail}");
            }
            other => panic!("expected an envelope error, got {other:?}"),
        }
    }
}
