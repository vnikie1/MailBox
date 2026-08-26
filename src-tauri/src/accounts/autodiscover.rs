//! Working out a domain's mail servers so the user does not have to.
//!
//! docs/04 Phase 4 names four sources, tried in this order because that is the order of
//! decreasing confidence:
//!
//! 1. **Mozilla's ISPDB** — a curated database covering most of the world's mail providers.
//! 2. **`autoconfig.<domain>` / `<domain>/.well-known/autoconfig`** — the domain answering
//!    for itself, which beats a third party's guess when the two disagree.
//! 3. **SRV records** (RFC 6186) — `_imaps._tcp`, `_submission._tcp`.
//! 4. **Port probing** — `imap.<domain>:993` and friends, a guess with a TCP handshake
//!    behind it.
//!
//! Every result says where it came from. A user looking at prefilled server settings should
//! be able to tell "Mozilla's database says so" from "we guessed and the port answered",
//! because those warrant different levels of trust when the connection then fails.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use ts_rs::TS;

use super::provider::{Security, ServerSettings};

/// Whole-lookup budget. Autodiscovery runs while the user watches a spinner, and four
/// sources at ten seconds each is a minute of nothing happening.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// Probing is the last resort and the slowest, so its per-port budget is tighter — a
/// firewall that drops packets rather than refusing them would otherwise hold the whole
/// lookup open.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum DiscoverySource {
    /// The provider was recognised outright; no lookup needed.
    Known,
    Ispdb,
    Autoconfig,
    SrvRecord,
    Probe,
}

impl DiscoverySource {
    /// A sentence for the settings pane, so prefilled fields say where they came from.
    pub fn explain(self) -> &'static str {
        match self {
            DiscoverySource::Known => "Halcyon knows this provider's servers.",
            DiscoverySource::Ispdb => "Found in Mozilla's provider database.",
            DiscoverySource::Autoconfig => "Published by the mail domain itself.",
            DiscoverySource::SrvRecord => "Found in the domain's DNS records.",
            DiscoverySource::Probe => {
                "Guessed from the domain name — the server answered, but check it is right."
            }
        }
    }

    /// Whether the user should look before accepting. Only probing is a guess.
    pub fn needs_confirmation(self) -> bool {
        self == DiscoverySource::Probe
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub imap: ServerSettings,
    pub smtp: ServerSettings,
    pub source: DiscoverySource,
    /// Set when the domain's own configuration says OAuth rather than a password — some
    /// hosted domains are Google or Microsoft behind a custom address, and offering a
    /// password box there fails with no explanation.
    pub oauth_hint: Option<String>,
}

pub fn domain_of(email: &str) -> Option<String> {
    let (_, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_lowercase();

    if domain.is_empty() || !domain.contains('.') {
        return None;
    }

    Some(domain)
}

/// Parses an autoconfig XML document.
///
/// Hand-parsed rather than pulling in an XML crate: the shape needed is four fields from a
/// well-known schema, and the input is untrusted. A hand parser that only ever looks for
/// specific tags cannot be talked into expanding an entity or fetching a DTD.
fn parse_autoconfig(xml: &str) -> Option<Discovered> {
    fn section<'a>(xml: &'a str, kind: &str) -> Option<&'a str> {
        let open = format!("<{kind}Server");
        let close = format!("</{kind}Server>");
        let start = xml.find(&open)?;
        let end = xml[start..].find(&close)? + start;
        Some(&xml[start..end])
    }

    fn tag<'a>(section: &'a str, name: &str) -> Option<&'a str> {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = section.find(&open)? + open.len();
        let end = section[start..].find(&close)? + start;
        Some(section[start..end].trim())
    }

    fn settings(section: &str, fallback: Security) -> Option<ServerSettings> {
        let host = tag(section, "hostname")?.to_string();
        let port: u16 = tag(section, "port")?.parse().ok()?;

        let security = match tag(section, "socketType")
            .unwrap_or("")
            .to_ascii_uppercase()
            .as_str()
        {
            "SSL" => Security::Tls,
            "STARTTLS" => Security::StartTls,
            // "plain" appears in the wild. docs/05 §6 forbids an unencrypted connection to
            // a public host with no user-visible bypass, so it is read as the secure form
            // for the port and left to the connection test to reject if that is wrong.
            _ => fallback,
        };

        Some(ServerSettings {
            host,
            port,
            security,
        })
    }

    let incoming = section(xml, "incoming")?;
    let outgoing = section(xml, "outgoing")?;

    // A domain fronting Google or Microsoft advertises OAuth here. Missing it means
    // offering a password box for an account whose provider rejects passwords.
    let oauth_hint = [incoming, outgoing]
        .iter()
        .filter_map(|section| tag(section, "authentication"))
        .find(|value| value.to_ascii_lowercase().contains("oauth"))
        .map(|value| value.to_string());

    Some(Discovered {
        imap: settings(incoming, Security::Tls)?,
        smtp: settings(outgoing, Security::StartTls)?,
        source: DiscoverySource::Autoconfig,
        oauth_hint,
    })
}

async fn fetch(client: &reqwest::Client, url: &str) -> Option<String> {
    let response = tokio::time::timeout(STEP_TIMEOUT, client.get(url).send())
        .await
        .ok()?
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.text().await.ok()
}

async fn from_ispdb(client: &reqwest::Client, domain: &str) -> Option<Discovered> {
    let xml = fetch(
        client,
        &format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
    )
    .await?;

    let mut found = parse_autoconfig(&xml)?;
    found.source = DiscoverySource::Ispdb;
    Some(found)
}

async fn from_autoconfig(client: &reqwest::Client, domain: &str) -> Option<Discovered> {
    // Both spellings are in use. The well-known path is the newer convention; the
    // autoconfig subdomain is what most deployments actually have.
    for url in [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress=user@{domain}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ] {
        if let Some(xml) = fetch(client, &url).await {
            if let Some(found) = parse_autoconfig(&xml) {
                return Some(found);
            }
        }
    }

    None
}

async fn from_srv(domain: &str) -> Option<Discovered> {
    use hickory_resolver::TokioAsyncResolver;

    // The system resolver, and only the system resolver. Falling back to a hard-coded public
    // one would send the user's mail domain to a DNS server they did not choose, which
    // standing rule 16 rules out; skipping the SRV step and probing instead is the honest
    // behaviour when the machine's own configuration cannot be read.
    let Ok(resolver) = TokioAsyncResolver::tokio_from_system_conf() else {
        return None;
    };

    let lookup = |name: String| {
        let resolver = resolver.clone();
        async move {
            let response = tokio::time::timeout(STEP_TIMEOUT, resolver.srv_lookup(name))
                .await
                .ok()?
                .ok()?;

            // Lowest priority wins, which is what the RFC means by preference — not the
            // first record the resolver happened to return.
            let best = response
                .iter()
                .min_by_key(|record| record.priority())?
                .clone();

            let host = best.target().to_utf8().trim_end_matches('.').to_string();
            if host.is_empty() {
                return None;
            }

            Some((host, best.port()))
        }
    };

    let (imap_host, imap_port) = lookup(format!("_imaps._tcp.{domain}")).await?;
    let (smtp_host, smtp_port) = lookup(format!("_submission._tcp.{domain}"))
        .await
        .unwrap_or_else(|| (imap_host.replace("imap", "smtp"), 587));

    Some(Discovered {
        imap: ServerSettings {
            host: imap_host,
            port: imap_port,
            security: Security::Tls,
        },
        smtp: ServerSettings {
            host: smtp_host,
            security: if smtp_port == 465 {
                Security::Tls
            } else {
                Security::StartTls
            },
            port: smtp_port,
        },
        source: DiscoverySource::SrvRecord,
        oauth_hint: None,
    })
}

/// Does anything answer on this host and port?
///
/// A TCP handshake only. Opening a TLS session and reading a greeting would be stronger
/// evidence, but this runs against up to six candidates and the connection test that
/// follows does the real work.
async fn answers(host: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

async fn from_probe(domain: &str) -> Option<Discovered> {
    let imap_candidates = [
        (format!("imap.{domain}"), 993u16),
        (format!("mail.{domain}"), 993),
        (domain.to_string(), 993),
    ];

    let mut imap = None;
    for (host, port) in imap_candidates {
        if answers(&host, port).await {
            imap = Some(ServerSettings {
                host,
                port,
                security: Security::Tls,
            });
            break;
        }
    }

    let imap = imap?;

    let smtp_candidates = [
        (format!("smtp.{domain}"), 587u16),
        (format!("mail.{domain}"), 587),
        (format!("smtp.{domain}"), 465),
    ];

    let mut smtp = None;
    for (host, port) in smtp_candidates {
        if answers(&host, port).await {
            smtp = Some(ServerSettings {
                host,
                security: if port == 465 {
                    Security::Tls
                } else {
                    Security::StartTls
                },
                port,
            });
            break;
        }
    }

    Some(Discovered {
        imap,
        // A server that answers on IMAP but not on any submission port is common behind a
        // firewall; the conventional default is offered and the connection test says
        // whether it works, which is better than abandoning the whole lookup.
        smtp: smtp.unwrap_or(ServerSettings {
            host: format!("smtp.{domain}"),
            port: 587,
            security: Security::StartTls,
        }),
        source: DiscoverySource::Probe,
        oauth_hint: None,
    })
}

/// Runs the four sources in order and returns the first that answers.
///
/// Sequential rather than concurrent on purpose: the order *is* the preference, and running
/// them together would mean either waiting for all of them or taking whichever was fastest,
/// which is not the same as taking the most trustworthy.
pub async fn discover(email: &str) -> Option<Discovered> {
    let domain = domain_of(email)?;

    let client = reqwest::Client::builder()
        .user_agent("Halcyon")
        // docs/05 §6: certificate validation stays on, with no bypass. A domain whose
        // autoconfig is served under a bad certificate is not a domain to take server
        // settings from.
        .timeout(STEP_TIMEOUT)
        .build()
        .ok()?;

    if let Some(found) = from_ispdb(&client, &domain).await {
        return Some(found);
    }

    if let Some(found) = from_autoconfig(&client, &domain).await {
        return Some(found);
    }

    if let Some(found) = from_srv(&domain).await {
        return Some(found);
    }

    from_probe(&domain).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_domain_is_taken_from_the_last_at_sign() {
        // Local parts may contain an @ when quoted. Splitting on the first would give a
        // domain of "b@example.test" and every lookup would fail.
        assert_eq!(
            domain_of("ada@example.test").as_deref(),
            Some("example.test")
        );
        assert_eq!(
            domain_of("\"a@b\"@example.test").as_deref(),
            Some("example.test")
        );
        assert_eq!(
            domain_of("  Ada@Example.TEST  ").as_deref(),
            Some("example.test")
        );
        assert_eq!(
            domain_of("ada@example.test.").as_deref(),
            Some("example.test")
        );
    }

    #[test]
    fn addresses_without_a_usable_domain_are_rejected() {
        assert_eq!(domain_of("no-at-sign"), None);
        assert_eq!(domain_of("ada@"), None);
        // A bare hostname cannot be looked up as a mail domain, and probing "localhost"
        // would be a request to a machine the user did not name.
        assert_eq!(domain_of("ada@localhost"), None);
    }

    #[test]
    fn autoconfig_xml_is_parsed_into_both_servers() {
        let xml = r#"
            <clientConfig version="1.1">
              <emailProvider id="example.test">
                <incomingServer type="imap">
                  <hostname>imap.example.test</hostname>
                  <port>993</port>
                  <socketType>SSL</socketType>
                  <authentication>password-cleartext</authentication>
                </incomingServer>
                <outgoingServer type="smtp">
                  <hostname>smtp.example.test</hostname>
                  <port>587</port>
                  <socketType>STARTTLS</socketType>
                  <authentication>password-cleartext</authentication>
                </outgoingServer>
              </emailProvider>
            </clientConfig>
        "#;

        let found = parse_autoconfig(xml).expect("parsed");

        assert_eq!(found.imap.host, "imap.example.test");
        assert_eq!(found.imap.port, 993);
        assert_eq!(found.imap.security, Security::Tls);
        assert_eq!(found.smtp.host, "smtp.example.test");
        assert_eq!(found.smtp.port, 587);
        assert_eq!(found.smtp.security, Security::StartTls);
        assert_eq!(found.oauth_hint, None);
    }

    #[test]
    fn a_domain_fronting_an_oauth_provider_is_noticed() {
        // A custom domain hosted on Google rejects passwords. Without this hint the user is
        // shown a password box and told "authentication failed" with no way to act on it.
        let xml = r#"
            <clientConfig>
              <incomingServer type="imap">
                <hostname>imap.gmail.com</hostname><port>993</port>
                <socketType>SSL</socketType>
                <authentication>OAuth2</authentication>
              </incomingServer>
              <outgoingServer type="smtp">
                <hostname>smtp.gmail.com</hostname><port>587</port>
                <socketType>STARTTLS</socketType>
                <authentication>OAuth2</authentication>
              </outgoingServer>
            </clientConfig>
        "#;

        let found = parse_autoconfig(xml).expect("parsed");
        assert_eq!(found.oauth_hint.as_deref(), Some("OAuth2"));
    }

    #[test]
    fn a_plaintext_socket_type_is_read_as_the_secure_form_not_as_plaintext() {
        // docs/05 §6 forbids an unencrypted connection to a public host with no bypass.
        // Honouring socketType=plain here would open one silently.
        let xml = r#"
            <clientConfig>
              <incomingServer type="imap">
                <hostname>imap.example.test</hostname><port>143</port>
                <socketType>plain</socketType>
              </incomingServer>
              <outgoingServer type="smtp">
                <hostname>smtp.example.test</hostname><port>25</port>
                <socketType>plain</socketType>
              </outgoingServer>
            </clientConfig>
        "#;

        let found = parse_autoconfig(xml).expect("parsed");

        assert_eq!(found.imap.security, Security::Tls);
        assert_eq!(found.smtp.security, Security::StartTls);
    }

    #[test]
    fn a_truncated_document_yields_nothing_rather_than_half_a_configuration() {
        // Half a configuration would be prefilled into the form and fail with a confusing
        // error; nothing at all falls through to the next source, which is what we want.
        assert!(parse_autoconfig("<clientConfig><incomingServer type=\"imap\">").is_none());
        assert!(parse_autoconfig("").is_none());
        assert!(parse_autoconfig(
            "<clientConfig><incomingServer><hostname>h</hostname></incomingServer>\
             <outgoingServer><hostname>s</hostname><port>587</port></outgoingServer></clientConfig>"
        )
        .is_none());
    }

    #[test]
    fn only_a_probed_result_asks_the_user_to_check_it() {
        assert!(DiscoverySource::Probe.needs_confirmation());
        assert!(!DiscoverySource::Ispdb.needs_confirmation());
        assert!(!DiscoverySource::Autoconfig.needs_confirmation());

        for source in [
            DiscoverySource::Known,
            DiscoverySource::Ispdb,
            DiscoverySource::Autoconfig,
            DiscoverySource::SrvRecord,
            DiscoverySource::Probe,
        ] {
            assert!(!source.explain().is_empty(), "{source:?} needs a sentence");
        }
    }
}
