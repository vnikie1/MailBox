//! A live check against a real account. `#[ignore]`d — it needs a configured account and a
//! network, so it never runs in CI or in `npm run verify`.
//!
//! Run with:
//!   cargo test --test live_gmail -- --ignored --nocapture
//!
//! It exists because a sync that *hangs* tells you nothing. The engine's own logging showed
//! only "sync starting" and then a sixty-second timeout, which narrows the fault to
//! "somewhere in the handshake" — four round trips wide. This walks the same steps one at a
//! time, with its own per-step timeout, and says which one stopped.
//!
//! It prints no secret. The token is used and never displayed.

use std::time::{Duration, Instant};

use halcyon_lib::accounts::credentials::{self, Kind};
use halcyon_lib::accounts::provider::{AuthKind, Provider};
use halcyon_lib::accounts::store;
use halcyon_lib::db::Db;

fn mark(name: &str, started: Instant) {
    println!("  [{:>7.2}s] {name}", started.elapsed().as_secs_f64());
}

#[tokio::test]
#[ignore = "needs a real configured account and a network"]
async fn walk_the_handshake() {
    let path = halcyon_lib::db::default_path();
    println!("store: {}", path.display());

    let db = Db::open(&path).expect("open store");
    let accounts = db.read(store::list).await.expect("list accounts");

    for account in accounts {
        let Some(imap) = account.imap.clone() else {
            println!("\n{} — no server configured, skipping", account.email);
            continue;
        };

        println!("\n=== {} ({}) ===", account.email, account.provider);
        println!("  imap {}:{}", imap.host, imap.port);

        let started = Instant::now();
        let reference = credentials::reference_for(&account.email);

        // ---- the credential --------------------------------------------------------------
        let token = match account.auth_kind {
            AuthKind::Password => match credentials::load(&reference, Kind::Password) {
                Ok(secret) => {
                    mark("password loaded", started);
                    Some(secret)
                }
                Err(error) => {
                    println!("  no password: {error}");
                    continue;
                }
            },

            AuthKind::OAuth2 => {
                let provider = Provider::from_id(&account.provider).unwrap_or(Provider::Other);

                let client = db
                    .read(move |conn| halcyon_lib::accounts::client_config(conn, provider))
                    .await
                    .expect("client config");

                let Some(client) = client else {
                    println!("  no OAuth client configured");
                    continue;
                };

                println!(
                    "  oauth client id set: yes, client secret set: {}",
                    client.client_secret.is_some()
                );

                let expiry = {
                    let reference = reference.clone();
                    db.read(move |conn| Ok(halcyon_lib::accounts::read_expiry(conn, &reference)))
                        .await
                        .expect("expiry")
                };

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                println!(
                    "  stored expiry {expiry} (now {now}; {}s away)",
                    expiry - now
                );

                match halcyon_lib::accounts::access_token(expiry, provider, &client, &reference)
                    .await
                {
                    Ok((token, refreshed)) => {
                        mark(
                            &format!("access token obtained (refreshed: {})", refreshed.is_some()),
                            started,
                        );
                        Some(token)
                    }
                    Err(error) => {
                        println!("  token failed: {error}");
                        continue;
                    }
                }
            }
        };

        let Some(token) = token else { continue };

        // ---- TCP ---------------------------------------------------------------------------
        let tcp = match tokio::time::timeout(
            Duration::from_secs(15),
            tokio::net::TcpStream::connect((imap.host.as_str(), imap.port)),
        )
        .await
        {
            Ok(Ok(stream)) => {
                mark("tcp connected", started);
                stream
            }
            other => {
                println!("  tcp failed: {other:?}");
                continue;
            }
        };

        // ---- TLS ---------------------------------------------------------------------------
        let tls = match tokio::time::timeout(
            Duration::from_secs(15),
            async_native_tls::TlsConnector::new().connect(imap.host.as_str(), tcp),
        )
        .await
        {
            Ok(Ok(stream)) => {
                mark("tls established", started);
                stream
            }
            other => {
                println!("  tls failed: {other:?}");
                continue;
            }
        };

        // ---- greeting + auth ----------------------------------------------------------------
        let mut client = async_imap::Client::new(tls);

        // The greeting. `Client::new` does not consume it, and leaving it unread is what
        // made AUTH hang: the auth loop reads it as the answer to its own command.
        match tokio::time::timeout(Duration::from_secs(10), client.read_response()).await {
            Ok(Some(Ok(response))) => {
                mark(&format!("greeting: {:?}", response.parsed()), started);
            }
            Ok(Some(Err(error))) => {
                println!("  greeting failed: {error}");
                continue;
            }
            Ok(None) => {
                println!("  connection closed before the greeting");
                continue;
            }
            Err(_) => {
                println!("  *** GREETING TIMED OUT ***");
                continue;
            }
        }

        let authenticated = match account.auth_kind {
            AuthKind::Password => tokio::time::timeout(
                Duration::from_secs(20),
                client.login(&account.email, token.expose()),
            )
            .await
            .map(|result| result.map_err(|(error, _)| error.to_string())),

            AuthKind::OAuth2 => {
                struct Auth {
                    email: String,
                    token: String,
                    sent: bool,
                }

                impl async_imap::Authenticator for Auth {
                    type Response = String;

                    fn process(&mut self, challenge: &[u8]) -> String {
                        if self.sent {
                            // The server's error payload, which is the only place it says
                            // *why* it refused. Not a secret — it is Google's own message.
                            println!("  server challenge: {}", String::from_utf8_lossy(challenge));
                            return String::new();
                        }

                        self.sent = true;
                        format!("user={}\x01auth=Bearer {}\x01\x01", self.email, self.token)
                    }
                }

                let auth = Auth {
                    email: account.email.clone(),
                    token: token.expose().to_string(),
                    sent: false,
                };

                tokio::time::timeout(
                    Duration::from_secs(20),
                    client.authenticate("XOAUTH2", auth),
                )
                .await
                .map(|result| result.map_err(|(error, _)| error.to_string()))
            }
        };

        let mut session = match authenticated {
            Ok(Ok(session)) => {
                mark("AUTHENTICATED", started);
                session
            }
            Ok(Err(error)) => {
                println!("  auth rejected: {error}");
                continue;
            }
            Err(_) => {
                println!("  *** AUTH TIMED OUT — this is where it hangs ***");
                continue;
            }
        };

        // ---- capabilities ---------------------------------------------------------------------
        match tokio::time::timeout(Duration::from_secs(15), session.capabilities()).await {
            Ok(Ok(caps)) => {
                let names: Vec<String> = caps.iter().map(|c| format!("{c:?}")).collect();
                mark(&format!("capabilities: {}", names.join(" ")), started);
            }
            Ok(Err(error)) => println!("  capabilities failed: {error}"),
            Err(_) => println!("  *** CAPABILITIES TIMED OUT ***"),
        }

        // ---- LIST ------------------------------------------------------------------------------
        let listing = tokio::time::timeout(Duration::from_secs(20), async {
            use futures::StreamExt;

            let mut stream = session.list(Some(""), Some("*")).await?;
            let mut names = Vec::new();

            while let Some(item) = stream.next().await {
                names.push(item?.name().to_string());
            }

            Ok::<_, async_imap::error::Error>(names)
        })
        .await;

        match listing {
            Ok(Ok(names)) => {
                mark(&format!("LIST returned {} mailboxes", names.len()), started);
                for name in names.iter().take(15) {
                    println!("      {name}");
                }
            }
            Ok(Err(error)) => println!("  LIST failed: {error}"),
            Err(_) => println!("  *** LIST TIMED OUT ***"),
        }

        let _ = session.logout().await;
    }
}

/// Re-parses every cached `.eml` without going near the network.
///
/// Run with:
///   cargo test --test live_gmail reparse -- --ignored --nocapture
///
/// This exists because the parser will keep improving — the first version leaked Outlook
/// conditional comments and numeric entities straight into the preview column — and the raw
/// source is already on disk. Re-deriving from the cache costs nothing and asks the mail
/// server for nothing; re-downloading a mailbox to fix a text bug would be absurd.
#[tokio::test]
#[ignore = "maintenance tool: re-derives stored text from the on-disk cache"]
async fn reparse_cached_bodies() {
    use halcyon_lib::sync::bodies;

    let path = halcyon_lib::db::default_path();
    let root = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let db = Db::open(&path).expect("open store");

    let cached: Vec<(i64, String)> = db
        .read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id, raw_path FROM message
                  WHERE body_state = 'full' AND raw_path IS NOT NULL",
            )?;

            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .expect("read cached");

    println!("cache root: {}", root.display());
    println!("{} cached bodies to re-parse", cached.len());

    let mut reparsed = 0usize;
    let mut missing = 0usize;

    for (message_id, raw_path) in cached {
        let Ok(raw) = std::fs::read(&raw_path) else {
            missing += 1;
            continue;
        };

        let body = bodies::parse(&raw);
        let stored = std::path::PathBuf::from(&raw_path);

        db.write(move |tx| bodies::persist(tx, message_id, &body, Some(&stored)))
            .await
            .expect("persist");

        reparsed += 1;
    }

    println!("re-parsed {reparsed}, {missing} cache files missing");

    // Show what the previews look like now, which is the whole point of running this.
    let samples: Vec<(String, String)> = db
        .read(|conn| {
            let mut statement = conn.prepare(
                "SELECT COALESCE(from_name, from_addr, ''), COALESCE(preview, '')
                   FROM message
                  WHERE body_state = 'full'
                  ORDER BY date_received DESC
                  LIMIT 8",
            )?;

            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .expect("samples");

    for (sender, preview) in samples {
        let shortened: String = preview.chars().take(90).collect();
        println!("  {sender:<24} | {shortened}");
    }
}

/// Prints what the renderer actually produces for the newest cached messages.
///
/// Run with:
///   cargo test --test live_gmail render_probe -- --ignored --nocapture
///
/// A body that renders as an empty white box tells you nothing about *which* step produced
/// nothing — the stored HTML, the sanitiser, or the image rewriter. This shows all three.
#[tokio::test]
#[ignore = "diagnostic: shows what the renderer does to real stored messages"]
async fn render_probe() {
    use halcyon_lib::mail::render;

    let db = Db::open(&halcyon_lib::db::default_path()).expect("open store");

    let rows: Vec<(i64, String, Option<String>, Option<String>)> = db
        .read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id, COALESCE(from_name, from_addr, '?'), body_html, body_text
                   FROM message
                  WHERE body_state = 'full'
                  ORDER BY date_received DESC
                  LIMIT 10",
            )?;

            let rows = statement.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;

            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .expect("read");

    println!("{} messages with a stored body\n", rows.len());

    for (id, sender, html, text) in rows {
        let rendered = render::render(
            html.as_deref(),
            text.as_deref(),
            &std::collections::HashMap::new(),
            false,
            &std::collections::HashMap::new(),
        );

        println!(
            "{id:>7} {sender:<26} html={:>7} text={:>6} -> out={:>7} blocked={} plain={}",
            html.as_deref().map(str::len).unwrap_or(0),
            text.as_deref().map(str::len).unwrap_or(0),
            rendered.html.len(),
            rendered.blocked_remote,
            rendered.from_plain_text,
        );

        if rendered.html.len() < 200 && html.as_deref().map(str::len).unwrap_or(0) > 500 {
            println!("        *** the sanitiser emptied a large body ***");
            println!(
                "        stored starts: {}",
                &html.as_deref().unwrap_or("")[..300.min(html.as_deref().unwrap_or("").len())]
            );
            println!("        output: {}", rendered.html);
        }
    }
}
