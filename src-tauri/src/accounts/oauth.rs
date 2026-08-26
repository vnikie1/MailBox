//! OAuth 2.0 with PKCE, in the system browser. docs/03 §7, docs/05 §2.
//!
//! Hand-rolled rather than pulled from a framework, because the interesting parts are ours
//! anyway: the loopback listener has to be, and PKCE is a hash and a redirect.
//!
//! Three things here are not negotiable, and each has a comment saying why:
//!
//! 1. **The system browser, never an embedded WebView.** docs/05 §2 — Google blocks embedded
//!    user agents with `disallowed_useragent`, and it is also the only arrangement where the
//!    user can see the address bar and know they are typing their password into Google.
//! 2. **PKCE, always.** A desktop client cannot keep a secret, so the client secret is not a
//!    secret. PKCE is what stops an authorisation code intercepted on the loopback redirect
//!    from being redeemable by anything else.
//! 3. **A `state` parameter, checked.** Without it the loopback listener accepts a code from
//!    any page that can reach localhost.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use ts_rs::TS;

use super::credentials::Secret;
use super::provider::Provider;

/// How long to wait for the user to finish in the browser before giving up.
///
/// Five minutes is long enough to find a password manager, read a consent screen and pass a
/// second factor. Shorter is a trap; longer leaves a listener open on the loopback interface
/// for no reason.
const AUTHORISATION_TIMEOUT: Duration = Duration::from_secs(300);

/// Refresh this long before the token actually expires. docs/03 §7.
///
/// Five minutes covers a slow network and a clock that disagrees with the server's by a
/// couple of minutes, which is common enough on desktops.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("{provider} does not use OAuth")]
    NotSupported { provider: String },

    #[error("no OAuth client is configured for {provider}")]
    NoClient { provider: String },

    #[error("could not open a loopback listener: {0}")]
    Listener(#[source] std::io::Error),

    #[error("could not open the browser: {0}")]
    Browser(#[source] std::io::Error),

    #[error("timed out waiting for the browser sign-in to finish")]
    TimedOut,

    /// The provider said no. `error` is its machine-readable code, which the caller branches
    /// on — `invalid_grant` in particular means re-authenticate, not retry.
    #[error("{provider} refused the request: {error}")]
    Refused {
        provider: String,
        error: String,
        description: Option<String>,
    },

    #[error("the redirect did not match the request it was started for")]
    StateMismatch,

    #[error("network error talking to {provider}: {source}")]
    Network {
        provider: String,
        #[source]
        source: reqwest::Error,
    },
}

/// The client registration this app uses for a provider.
///
/// Both fields come from the user or from configuration; nothing is compiled in. docs/05 §2
/// recommends a bring-your-own-client option for exactly this, and having no other path
/// means the app is usable before Google's verification completes.
#[derive(Clone)]
pub struct ClientConfig {
    pub client_id: String,
    /// Present for Google (which issues one even for desktop clients) and absent for
    /// Microsoft public clients. Not a secret in the usual sense — a desktop app cannot keep
    /// one — which is exactly why PKCE is mandatory rather than optional.
    pub client_secret: Option<Secret>,
}

/// A token set as the rest of the app sees it. The refresh token never leaves this module
/// except on its way into the Credential Manager.
pub struct Tokens {
    pub access: Secret,
    pub refresh: Option<Secret>,
    /// Absolute epoch seconds, not a duration — a duration would go stale the moment it was
    /// stored, and it is stored.
    pub expires_at: i64,
}

/// What the UI shows while the browser is open.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PendingAuthorisation {
    pub url: String,
    pub redirect_uri: String,
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn random_urlsafe(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer)
}

/// A PKCE pair. The verifier is held in memory only for the length of one sign-in.
pub struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        // 64 bytes → 86 base64url characters, inside RFC 7636's 43–128 range with room to
        // spare, and well beyond guessing.
        let verifier = random_urlsafe(64);
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

        Self {
            verifier,
            challenge,
        }
    }

    pub fn challenge(&self) -> &str {
        &self.challenge
    }

    pub fn verifier(&self) -> &str {
        &self.verifier
    }
}

/// The response the browser is left looking at.
///
/// Deliberately a complete little page rather than a bare word: this is the last thing the
/// user sees of the sign-in, and a blank tab reading "ok" makes people wonder whether it
/// worked. No external resources, because it is served from a loopback socket that closes
/// a moment later.
fn completion_page(success: bool) -> String {
    let (title, message) = if success {
        ("Signed in", "You can close this tab and return to Halcyon.")
    } else {
        (
            "Sign-in failed",
            "Halcyon did not receive an authorisation code. Return to the app and try again.",
        )
    };

    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>{title}</title><style>\
         body{{font:16px/1.5 'Segoe UI Variable Text','Segoe UI',system-ui,sans-serif;\
         display:grid;place-items:center;height:100vh;margin:0;color:#1c1c1e;background:#fff}}\
         main{{text-align:center;max-width:32rem;padding:2rem}}\
         h1{{font-size:1.4rem;font-weight:600;margin:0 0 .5rem}}\
         p{{margin:0;color:#3c3c43}}\
         @media(prefers-color-scheme:dark){{body{{color:#f2f2f7;background:#1e1e1e}}\
         p{{color:#aeaeb2}}}}</style></head>\
         <body><main><h1>{title}</h1><p>{message}</p></main></body></html>"
    );

    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Waits for the provider to redirect back, and returns the authorisation code.
///
/// Binds to 127.0.0.1 on a port the OS chooses. A fixed port would collide with whatever
/// else is running and, worse, would let another local process squat on it and receive the
/// code first.
async fn await_code(listener: TcpListener, expected_state: &str) -> Result<String, OAuthError> {
    let accept = async {
        loop {
            let (mut stream, _) = listener.accept().await.map_err(OAuthError::Listener)?;

            // Only the request line is needed, and reading exactly one line means a client
            // that opens a connection and says nothing cannot hold the listener open.
            let mut reader = BufReader::new(&mut stream);
            let mut request_line = String::new();
            let read = reader.read_line(&mut request_line).await;

            if read.is_err() || request_line.is_empty() {
                continue;
            }

            let target = request_line.split_whitespace().nth(1).unwrap_or("/");
            let url = url::Url::parse(&format!("http://127.0.0.1{target}"));

            let Ok(url) = url else {
                continue;
            };

            // Browsers ask for /favicon.ico on the side; answering the wrong request would
            // abandon the sign-in.
            if url.path() != "/callback" {
                let _ = stream.write_all(completion_page(false).as_bytes()).await;
                let _ = stream.shutdown().await;
                continue;
            }

            let mut code = None;
            let mut state = None;
            let mut error = None;
            let mut description = None;

            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "code" => code = Some(value.into_owned()),
                    "state" => state = Some(value.into_owned()),
                    "error" => error = Some(value.into_owned()),
                    "error_description" => description = Some(value.into_owned()),
                    _ => {}
                }
            }

            let success = code.is_some() && error.is_none();
            let _ = stream.write_all(completion_page(success).as_bytes()).await;
            let _ = stream.shutdown().await;

            if let Some(error) = error {
                return Err(OAuthError::Refused {
                    provider: "the provider".into(),
                    error,
                    description,
                });
            }

            // Checked before the code is used for anything. Without this the listener would
            // accept a code from any page on the machine that can reach localhost.
            if state.as_deref() != Some(expected_state) {
                return Err(OAuthError::StateMismatch);
            }

            if let Some(code) = code {
                return Ok(code);
            }
        }
    };

    match tokio::time::timeout(AUTHORISATION_TIMEOUT, accept).await {
        Ok(result) => result,
        Err(_) => Err(OAuthError::TimedOut),
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
    error_description: Option<String>,
}

async fn exchange(
    provider: Provider,
    client: &ClientConfig,
    form: Vec<(&str, String)>,
) -> Result<Tokens, OAuthError> {
    let endpoint = provider
        .token_endpoint()
        .ok_or_else(|| OAuthError::NotSupported {
            provider: provider.id().to_string(),
        })?;

    let mut form = form;
    form.push(("client_id", client.client_id.clone()));
    if let Some(secret) = &client.client_secret {
        form.push(("client_secret", secret.expose().to_string()));
    }

    // A token request with no ceiling can hang the sync engine forever behind an account
    // lock. Everything that touches a network in this app has a timeout.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|source| OAuthError::Network {
            provider: provider.id().to_string(),
            source,
        })?;

    let response = client
        .post(endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|source| OAuthError::Network {
            provider: provider.id().to_string(),
            source,
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|source| OAuthError::Network {
            provider: provider.id().to_string(),
            source,
        })?;

    if !status.is_success() {
        // The body is parsed for the machine-readable code rather than surfaced raw: it is
        // what tells `invalid_grant` (re-authenticate) from a transient failure (retry).
        let parsed: Result<TokenErrorResponse, _> = serde_json::from_str(&body);
        return Err(match parsed {
            Ok(error) => OAuthError::Refused {
                provider: provider.id().to_string(),
                error: error.error,
                description: error.error_description,
            },
            Err(_) => OAuthError::Refused {
                provider: provider.id().to_string(),
                error: format!("http_{}", status.as_u16()),
                description: None,
            },
        });
    }

    let tokens: TokenResponse = serde_json::from_str(&body).map_err(|_| OAuthError::Refused {
        provider: provider.id().to_string(),
        error: "malformed_token_response".into(),
        description: None,
    })?;

    Ok(Tokens {
        access: Secret::new(tokens.access_token),
        refresh: tokens.refresh_token.map(Secret::new),
        // Default to an hour when the provider omits it, which is the common default and
        // errs toward refreshing too often rather than too late.
        expires_at: now_seconds() + tokens.expires_in.unwrap_or(3600),
    })
}

/// Runs a full sign-in: opens the browser, waits for the redirect, exchanges the code.
pub async fn authorise(
    provider: Provider,
    client: &ClientConfig,
    login_hint: Option<&str>,
    open_browser: impl FnOnce(&str) -> Result<(), std::io::Error>,
) -> Result<Tokens, OAuthError> {
    let authorize = provider
        .authorize_endpoint()
        .ok_or_else(|| OAuthError::NotSupported {
            provider: provider.id().to_string(),
        })?;

    if client.client_id.trim().is_empty() {
        return Err(OAuthError::NoClient {
            provider: provider.id().to_string(),
        });
    }

    // Port 0 asks the OS for a free one. Bound before the browser opens, so the redirect
    // cannot arrive before anything is listening.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(OAuthError::Listener)?;
    let port = listener.local_addr().map_err(OAuthError::Listener)?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let pkce = Pkce::generate();
    let state = random_urlsafe(24);

    let mut url = url::Url::parse(authorize).map_err(|_| OAuthError::NotSupported {
        provider: provider.id().to_string(),
    })?;

    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &provider.scopes().join(" "))
            .append_pair("state", &state)
            .append_pair("code_challenge", pkce.challenge())
            .append_pair("code_challenge_method", "S256");

        // Google only returns a refresh token when both are present, and only on the first
        // consent — without them the account silently stops working in an hour.
        if provider == Provider::Google {
            query
                .append_pair("access_type", "offline")
                .append_pair("prompt", "consent");
        }

        // Pre-fills the account, so a user with several signs in to the one they meant.
        if let Some(hint) = login_hint {
            query.append_pair("login_hint", hint);
        }
    }

    open_browser(url.as_str()).map_err(OAuthError::Browser)?;

    let code = await_code(listener, &state).await?;

    exchange(
        provider,
        client,
        vec![
            ("grant_type", "authorization_code".into()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", pkce.verifier().to_string()),
        ],
    )
    .await
}

/// Exchanges a refresh token for a fresh access token.
///
/// Providers often omit a new refresh token, meaning "keep the one you have" — returning
/// `None` there and having the caller keep the stored one is the difference between an
/// account that keeps working and one that logs itself out.
pub async fn refresh(
    provider: Provider,
    client: &ClientConfig,
    refresh_token: &Secret,
) -> Result<Tokens, OAuthError> {
    exchange(
        provider,
        client,
        vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh_token.expose().to_string()),
        ],
    )
    .await
}

/// Whether a token should be refreshed now. docs/03 §7 — five minutes before expiry.
pub fn needs_refresh(expires_at: i64) -> bool {
    now_seconds() + REFRESH_MARGIN.as_secs() as i64 >= expires_at
}

/// Whether this failure means the user must sign in again rather than the app retrying.
///
/// docs/03 §7: on `invalid_grant`, surface a re-authenticate banner rather than failing
/// silently. Everything else is transient and worth retrying.
pub fn requires_reauthentication(error: &OAuthError) -> bool {
    matches!(
        error,
        OAuthError::Refused { error, .. }
            if error == "invalid_grant" || error == "invalid_client" || error == "unauthorized_client"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_request_has_a_timeout() {
        // Not a property of a value, so asserted at the source: a token request without a
        // ceiling hangs the account it belongs to, and an account lock held forever is how a
        // sync engine stops working with nothing in the log.
        let source = include_str!("oauth.rs");

        assert!(
            source.contains(".timeout(Duration::from_secs("),
            "the OAuth HTTP client must set a timeout"
        );
    }

    #[test]
    fn pkce_challenge_is_the_sha256_of_the_verifier_base64url_unpadded() {
        // RFC 7636. Getting the encoding wrong (padding, or standard base64) fails at the
        // token endpoint with a message that does not say which end is wrong.
        let pkce = Pkce::generate();

        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(pkce.verifier().as_bytes()));

        assert_eq!(pkce.challenge(), expected);
        assert!(!pkce.challenge().contains('='), "must be unpadded");
        assert!(!pkce.challenge().contains('+') && !pkce.challenge().contains('/'));
    }

    #[test]
    fn verifiers_are_long_and_never_repeat() {
        let a = Pkce::generate();
        let b = Pkce::generate();

        assert_ne!(a.verifier(), b.verifier());
        // RFC 7636 requires 43..=128 characters.
        assert!(
            (43..=128).contains(&a.verifier().len()),
            "{}",
            a.verifier().len()
        );
    }

    #[test]
    fn a_token_that_expires_soon_is_refreshed_early() {
        // docs/03 §7 — five minutes of margin, so a slow network or a skewed clock does not
        // produce a request with an already-dead token.
        assert!(needs_refresh(now_seconds() + 60));
        assert!(needs_refresh(now_seconds() + 299));
        assert!(!needs_refresh(now_seconds() + 3600));
    }

    #[test]
    fn invalid_grant_means_sign_in_again_and_a_network_blip_does_not() {
        let refused = |code: &str| OAuthError::Refused {
            provider: "google".into(),
            error: code.into(),
            description: None,
        };

        assert!(requires_reauthentication(&refused("invalid_grant")));
        assert!(requires_reauthentication(&refused("invalid_client")));
        assert!(!requires_reauthentication(&refused(
            "temporarily_unavailable"
        )));
        assert!(!requires_reauthentication(&OAuthError::TimedOut));
    }

    #[test]
    fn the_completion_page_pulls_in_nothing_external() {
        // It is served from a loopback socket that closes immediately; anything remote
        // would render as a broken page, and any script would be a needless attack surface.
        let page = completion_page(true);

        assert!(page.contains("Content-Length"));
        assert!(!page.contains("http://") && !page.contains("https://"));
        assert!(!page.contains("<script"));
    }

    #[tokio::test]
    async fn the_listener_rejects_a_code_with_the_wrong_state() {
        // Without the state check, any page on this machine that can reach the loopback port
        // could hand us a code of its choosing.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let waiter = tokio::spawn(async move { await_code(listener, "expected-state").await });

        let response = reqwest::get(format!(
            "http://127.0.0.1:{port}/callback?code=stolen&state=attacker"
        ))
        .await;
        assert!(
            response.is_ok(),
            "the listener should still answer the request"
        );

        match waiter.await.expect("join") {
            Err(OAuthError::StateMismatch) => {}
            other => panic!("expected StateMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_listener_ignores_a_favicon_request_and_keeps_waiting() {
        // Browsers ask for /favicon.ico unprompted. Treating that as the callback would
        // abandon the sign-in for a request the user never made.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let waiter = tokio::spawn(async move { await_code(listener, "state-1").await });

        let _ = reqwest::get(format!("http://127.0.0.1:{port}/favicon.ico")).await;
        let _ = reqwest::get(format!(
            "http://127.0.0.1:{port}/callback?code=good-code&state=state-1"
        ))
        .await;

        let code = waiter.await.expect("join").expect("code");
        assert_eq!(code, "good-code");
    }

    #[tokio::test]
    async fn a_provider_error_on_the_redirect_is_reported_rather_than_ignored() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let waiter = tokio::spawn(async move { await_code(listener, "state-2").await });

        let _ = reqwest::get(format!(
            "http://127.0.0.1:{port}/callback?error=access_denied&error_description=User%20said%20no&state=state-2"
        ))
        .await;

        match waiter.await.expect("join") {
            Err(OAuthError::Refused {
                error, description, ..
            }) => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("User said no"));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn authorise_refuses_before_opening_a_browser_when_no_client_is_configured() {
        // Opening a browser to an authorize URL with an empty client_id shows the user a
        // Google error page, which reads as the app being broken rather than unconfigured.
        let client = ClientConfig {
            client_id: "  ".into(),
            client_secret: None,
        };

        let mut opened = false;
        let result = authorise(Provider::Google, &client, None, |_| {
            opened = true;
            Ok(())
        })
        .await;

        assert!(matches!(result, Err(OAuthError::NoClient { .. })));
        assert!(!opened, "the browser must not open");
    }

    #[tokio::test]
    async fn a_provider_without_oauth_is_refused_rather_than_attempted() {
        let client = ClientConfig {
            client_id: "id".into(),
            client_secret: None,
        };

        let result = authorise(Provider::ICloud, &client, None, |_| Ok(())).await;
        assert!(matches!(result, Err(OAuthError::NotSupported { .. })));
    }

    #[tokio::test]
    async fn the_authorisation_url_carries_pkce_state_and_the_offline_flags() {
        let client = ClientConfig {
            client_id: "test-client".into(),
            client_secret: None,
        };

        // The browser callback captures the URL; the sign-in then times out on its own,
        // which is fine — the URL is what is under test.
        //
        // A tokio channel rather than a std one: `#[tokio::test]` runs on a current-thread
        // runtime, so a blocking `recv` here would park the only thread the spawned task
        // could run on, and the test would deadlock rather than fail.
        let (sender, receiver) = tokio::sync::oneshot::channel();

        let attempt = tokio::spawn(async move {
            authorise(
                Provider::Google,
                &client,
                Some("ada@example.test"),
                move |url| {
                    let _ = sender.send(url.to_string());
                    Ok(())
                },
            )
            .await
        });

        let url = tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .expect("the browser should have been opened")
            .expect("url");
        attempt.abort();

        let parsed = url::Url::parse(&url).expect("parse");
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert!(query.contains_key("code_challenge"));
        assert!(query.contains_key("state"));
        assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            query.get("scope").map(String::as_str),
            Some("https://mail.google.com/")
        );
        assert_eq!(
            query.get("login_hint").map(String::as_str),
            Some("ada@example.test")
        );

        // Without both of these Google issues no refresh token, and the account stops
        // working an hour later with no explanation.
        assert_eq!(
            query.get("access_type").map(String::as_str),
            Some("offline")
        );
        assert_eq!(query.get("prompt").map(String::as_str), Some("consent"));

        // The redirect must be loopback. docs/05 §2 — and a non-loopback redirect would be
        // rejected by Google for a desktop client anyway.
        let redirect = query.get("redirect_uri").expect("redirect_uri");
        assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
        assert!(redirect.ends_with("/callback"));
    }
}
