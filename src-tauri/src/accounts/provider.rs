//! What we know about each mail provider before the user types anything.
//!
//! docs/05 §2–§5 is the source: each provider has its own failure modes, and the value of a
//! provider picker is that it lets us say something specific instead of "authentication
//! failed". Every string in this file exists because a user would otherwise be stuck.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How an account proves who it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum AuthKind {
    /// OAuth 2.0 with PKCE, in the system browser. docs/03 §7.
    OAuth2,
    /// A password or an app-specific password, over TLS.
    Password,
}

/// Transport security for a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    /// TLS from the first byte. IMAP 993, SMTP 465.
    Tls,
    /// Plaintext connection upgraded with STARTTLS. IMAP 143, SMTP 587.
    StartTls,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

/// A provider the picker offers. docs/04 Phase 4 — Google, Microsoft, iCloud, Yahoo, Other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Google,
    Microsoft,
    ICloud,
    Yahoo,
    Other,
}

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Provider::Google => "google",
            Provider::Microsoft => "microsoft",
            Provider::ICloud => "icloud",
            Provider::Yahoo => "yahoo",
            Provider::Other => "other",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "google" | "gmail" => Some(Provider::Google),
            "microsoft" | "outlook" => Some(Provider::Microsoft),
            "icloud" => Some(Provider::ICloud),
            "yahoo" => Some(Provider::Yahoo),
            "other" | "imap" => Some(Provider::Other),
            _ => None,
        }
    }

    pub fn auth_kind(self) -> AuthKind {
        match self {
            // docs/05 §2 and §3: both mandate OAuth. Microsoft has disabled basic auth for
            // IMAP and SMTP outright, and Google blocks password auth for most accounts.
            Provider::Google | Provider::Microsoft => AuthKind::OAuth2,
            // docs/05 §4: Apple offers no third-party OAuth at all.
            Provider::ICloud => AuthKind::Password,
            Provider::Yahoo | Provider::Other => AuthKind::Password,
        }
    }

    /// Known-good servers, so autodiscovery is only needed for `Other`.
    pub fn servers(self) -> Option<(ServerSettings, ServerSettings)> {
        let pair = match self {
            Provider::Google => (
                ("imap.gmail.com", 993, Security::Tls),
                ("smtp.gmail.com", 587, Security::StartTls),
            ),
            Provider::Microsoft => (
                ("outlook.office365.com", 993, Security::Tls),
                ("smtp.office365.com", 587, Security::StartTls),
            ),
            // docs/05 §4 gives these exactly.
            Provider::ICloud => (
                ("imap.mail.me.com", 993, Security::Tls),
                ("smtp.mail.me.com", 587, Security::StartTls),
            ),
            Provider::Yahoo => (
                ("imap.mail.yahoo.com", 993, Security::Tls),
                ("smtp.mail.yahoo.com", 465, Security::Tls),
            ),
            Provider::Other => return None,
        };

        Some((
            ServerSettings {
                host: pair.0 .0.into(),
                port: pair.0 .1,
                security: pair.0 .2,
            },
            ServerSettings {
                host: pair.1 .0.into(),
                port: pair.1 .1,
                security: pair.1 .2,
            },
        ))
    }

    /// The OAuth scopes this provider needs for mail.
    pub fn scopes(self) -> &'static [&'static str] {
        match self {
            // docs/05 §2: the restricted scope. Anything narrower cannot read mail over IMAP.
            Provider::Google => &["https://mail.google.com/"],
            // docs/05 §3 lists these four exactly.
            Provider::Microsoft => &[
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "https://outlook.office.com/SMTP.Send",
                "offline_access",
                "User.Read",
            ],
            _ => &[],
        }
    }

    pub fn authorize_endpoint(self) -> Option<&'static str> {
        match self {
            Provider::Google => Some("https://accounts.google.com/o/oauth2/v2/auth"),
            // docs/05 §3: consumer accounts need the `common` (or `consumers`) tenant.
            Provider::Microsoft => {
                Some("https://login.microsoftonline.com/common/oauth2/v2.0/authorize")
            }
            _ => None,
        }
    }

    pub fn token_endpoint(self) -> Option<&'static str> {
        match self {
            Provider::Google => Some("https://oauth2.googleapis.com/token"),
            Provider::Microsoft => {
                Some("https://login.microsoftonline.com/common/oauth2/v2.0/token")
            }
            _ => None,
        }
    }

    /// Whether this provider refuses to refresh a token without a client secret.
    ///
    /// Google issues a secret even for "Desktop app" clients and requires it on every token
    /// exchange; Microsoft public clients have none and reject one if sent. Getting this
    /// wrong produces `invalid_request` from the token endpoint, which says nothing about
    /// which of the two mistakes was made.
    pub fn requires_client_secret(self) -> bool {
        matches!(self, Provider::Google)
    }

    /// How many connections this provider tolerates. docs/05 §5.
    ///
    /// Yahoo and AOL throttle aggressively, and being throttled looks to a user exactly like
    /// the app being broken.
    pub fn connection_limit(self) -> u8 {
        match self {
            Provider::Yahoo => 2,
            _ => 3,
        }
    }
}

/// What the UI needs to describe a provider, including the guidance docs/05 says is the
/// single most common support issue for every third-party mail client.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub auth_kind: AuthKind,
    /// Whether the app already knows the servers, or the user must supply them.
    pub needs_manual_setup: bool,
    /// Set where signing in needs a step outside this app.
    pub setup_note: Option<String>,
    /// Where that step happens, for a "Open in browser" button.
    pub setup_url: Option<String>,
    /// True when no OAuth client is configured, so the provider cannot be used yet.
    pub needs_oauth_client: bool,
    /// Whether this provider refuses to refresh a token without a client secret.
    ///
    /// Surfaced so the settings field can be labelled honestly. It was marked "(optional)"
    /// for every provider, which is true of Microsoft and false of Google — and the symptom
    /// of leaving it blank for Google is a token refresh failing an hour later with a
    /// message about the sign-in being rejected.
    pub requires_client_secret: bool,
}

pub fn describe(provider: Provider, has_oauth_client: bool) -> ProviderInfo {
    let (display_name, setup_note, setup_url) = match provider {
        Provider::Google => (
            "Google",
            Some(
                "Sign in happens in your browser. Halcyon never sees your Google password."
                    .to_string(),
            ),
            None,
        ),
        Provider::Microsoft => (
            "Microsoft",
            Some(
                "Sign in happens in your browser. If your work or school account fails, your \
                 administrator may have blocked IMAP for third-party apps."
                    .to_string(),
            ),
            None,
        ),
        // docs/05 §4: this is the flow to get right, because everyone gets stuck here.
        Provider::ICloud => (
            "iCloud",
            Some(
                "iCloud needs an app-specific password, not your Apple ID password. Sign in at \
                 appleid.apple.com, go to Sign-In and Security, choose App-Specific Passwords, \
                 and create one for Halcyon. Your Apple ID must have two-factor authentication \
                 turned on."
                    .to_string(),
            ),
            Some("https://appleid.apple.com/account/manage".to_string()),
        ),
        Provider::Yahoo => (
            "Yahoo",
            Some(
                "Yahoo needs an app password. Generate one under Account Security in your Yahoo \
                 account settings."
                    .to_string(),
            ),
            Some("https://login.yahoo.com/account/security".to_string()),
        ),
        Provider::Other => ("Other Mail Account", None, None),
    };

    ProviderInfo {
        id: provider.id().to_string(),
        display_name: display_name.to_string(),
        auth_kind: provider.auth_kind(),
        needs_manual_setup: provider.servers().is_none(),
        setup_note,
        setup_url,
        needs_oauth_client: provider.auth_kind() == AuthKind::OAuth2 && !has_oauth_client,
        requires_client_secret: provider.requires_client_secret(),
    }
}

pub const ALL: &[Provider] = &[
    Provider::Google,
    Provider::Microsoft,
    Provider::ICloud,
    Provider::Yahoo,
    Provider::Other,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_round_trips_through_its_id() {
        for provider in ALL {
            assert_eq!(Provider::from_id(provider.id()), Some(*provider));
        }
    }

    #[test]
    fn common_aliases_resolve() {
        // A stored account row may say "gmail" or "outlook" from an earlier version, or from
        // a user typing what they call it.
        assert_eq!(Provider::from_id("gmail"), Some(Provider::Google));
        assert_eq!(Provider::from_id("outlook"), Some(Provider::Microsoft));
        assert_eq!(Provider::from_id("imap"), Some(Provider::Other));
        assert_eq!(Provider::from_id("nonsense"), None);
    }

    #[test]
    fn the_oauth_providers_are_exactly_the_ones_that_mandate_it() {
        // docs/05 §2 and §3. Getting this wrong means offering a password box for an account
        // whose provider has disabled password auth, which fails with no explanation.
        assert_eq!(Provider::Google.auth_kind(), AuthKind::OAuth2);
        assert_eq!(Provider::Microsoft.auth_kind(), AuthKind::OAuth2);
        assert_eq!(Provider::ICloud.auth_kind(), AuthKind::Password);
    }

    #[test]
    fn google_requests_the_restricted_scope_because_nothing_narrower_reads_mail() {
        assert_eq!(Provider::Google.scopes(), &["https://mail.google.com/"]);
    }

    #[test]
    fn microsoft_requests_offline_access_or_there_is_no_refresh_token() {
        // Without offline_access the token expires in an hour and the user is asked to sign
        // in again every time — which reads as the app being broken.
        assert!(Provider::Microsoft.scopes().contains(&"offline_access"));
    }

    #[test]
    fn icloud_ships_the_app_specific_password_instructions() {
        // docs/05 §4 calls this the single most common support issue for third-party
        // clients. An empty note here is a real regression.
        let info = describe(Provider::ICloud, false);

        assert!(info.setup_note.is_some());
        assert!(info.setup_url.is_some());
        assert!(!info.needs_oauth_client, "iCloud needs no OAuth client");
        assert!(
            info.setup_note.unwrap_or_default().contains("app-specific"),
            "the note must name what the user has to generate"
        );
    }

    #[test]
    fn only_the_other_provider_asks_the_user_for_servers() {
        for provider in ALL {
            let manual = describe(*provider, true).needs_manual_setup;
            assert_eq!(manual, *provider == Provider::Other, "{provider:?}");
        }
    }

    #[test]
    fn an_oauth_provider_reports_a_missing_client() {
        assert!(describe(Provider::Google, false).needs_oauth_client);
        assert!(!describe(Provider::Google, true).needs_oauth_client);
        assert!(!describe(Provider::Other, false).needs_oauth_client);
    }

    #[test]
    fn google_needs_a_client_secret_and_microsoft_does_not() {
        // Google issues one even for a "Desktop app" client and requires it on every token
        // exchange; a Microsoft public client has none and rejects one if sent. The settings
        // field is labelled from this, because "(optional)" for Google is a lie whose cost
        // arrives an hour later as a refresh failure that reads like a bad password.
        assert!(describe(Provider::Google, true).requires_client_secret);
        assert!(!describe(Provider::Microsoft, true).requires_client_secret);
        assert!(!describe(Provider::ICloud, true).requires_client_secret);
    }

    #[test]
    fn yahoo_is_capped_lower_because_it_throttles() {
        // docs/05 §5. Being throttled looks exactly like the app being broken.
        assert_eq!(Provider::Yahoo.connection_limit(), 2);
        assert_eq!(Provider::Google.connection_limit(), 3);
    }
}
