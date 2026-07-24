//! Tauri correlation for Chill.
//!
//! This crate deliberately handles *only* declared metadata. It never reads,
//! formats, clones, hashes, measures, logs, or persists command/event payloads,
//! results, errors, or application arguments. Export is intentionally owned by
//! the host application's public Chill Rust SDK client.

use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const CARRIER_VERSION: u8 = 1;
pub const MAX_BAGGAGE_ENTRIES: usize = 8;
pub const MAX_BAGGAGE_KEY_BYTES: usize = 32;
pub const MAX_BAGGAGE_VALUE_BYTES: usize = 64;
pub const MAX_SEMANTIC_NAME_BYTES: usize = 80;

/// A renderer-provided, metadata-only correlation envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Carrier {
    pub version: u8,
    pub traceparent: String,
    pub baggage: BTreeMap<String, String>,
    pub semantic_name: String,
}

/// An opaque sender identity supplied by the integration boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebviewIdentity {
    pub origin: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedWebview {
    pub origin: String,
    pub label: String,
}

impl TrustedWebview {
    pub fn matches(&self, sender: &WebviewIdentity) -> bool {
        self.origin == sender.origin && self.label == sender.label
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidCarrierReason {
    Missing,
    UntrustedWebview,
    Version,
    Traceparent,
    Baggage,
    SemanticName,
    SemanticMismatch,
}

impl InvalidCarrierReason {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::UntrustedWebview => "untrusted_webview",
            Self::Version => "version",
            Self::Traceparent => "traceparent",
            Self::Baggage => "baggage",
            Self::SemanticName => "semantic_name",
            Self::SemanticMismatch => "semantic_mismatch",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Correlation {
    Remote(Carrier),
    Local { reason: InvalidCarrierReason },
}

/// Fixed, low-cardinality application configuration. Raw Tauri names are never
/// telemetry and must be explicitly mapped to a semantic name.
#[derive(Clone, Debug, Default)]
pub struct Correlator {
    commands: HashMap<String, String>,
    events: HashMap<String, String>,
    trusted: Vec<TrustedWebview>,
    baggage_allowlist: BTreeSet<String>,
}

impl Correlator {
    pub fn new(trusted: impl IntoIterator<Item = TrustedWebview>) -> Self {
        Self {
            trusted: trusted.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn command(
        mut self,
        raw_name: impl Into<String>,
        declared_name: impl Into<String>,
    ) -> Self {
        let semantic = declared_name.into();
        assert!(
            semantic_name(&semantic),
            "Tauri command semantic name must match the Chill contract"
        );
        self.commands.insert(raw_name.into(), semantic);
        self
    }

    pub fn event(mut self, raw_name: impl Into<String>, declared_name: impl Into<String>) -> Self {
        let semantic = declared_name.into();
        assert!(
            semantic_name(&semantic),
            "Tauri event semantic name must match the Chill contract"
        );
        self.events.insert(raw_name.into(), semantic);
        self
    }

    /// Allows a public, bounded baggage key. The default is empty: carrier
    /// baggage is rejected unless every key is explicitly declared here.
    pub fn allow_baggage_key(mut self, key: impl Into<String>) -> Self {
        self.baggage_allowlist.insert(key.into());
        self
    }

    pub fn command_correlation(
        &self,
        raw_name: &str,
        sender: &WebviewIdentity,
        carrier: Option<&Carrier>,
    ) -> Option<Correlation> {
        self.correlate(&self.commands, raw_name, sender, carrier)
    }

    /// Returns the declared semantic name together with command correlation.
    /// This is the consumer shape used by host SDK integrations; raw command
    /// names remain configuration and never become telemetry attributes.
    pub fn command_context(
        &self,
        raw_name: &str,
        sender: &WebviewIdentity,
        carrier: Option<&Carrier>,
    ) -> Option<(&str, Correlation)> {
        let semantic = self.commands.get(raw_name)?;
        self.command_correlation(raw_name, sender, carrier)
            .map(|correlation| (semantic.as_str(), correlation))
    }

    pub fn event_correlation(
        &self,
        raw_name: &str,
        sender: &WebviewIdentity,
        carrier: Option<&Carrier>,
    ) -> Option<Correlation> {
        self.correlate(&self.events, raw_name, sender, carrier)
    }

    fn correlate(
        &self,
        names: &HashMap<String, String>,
        raw: &str,
        sender: &WebviewIdentity,
        carrier: Option<&Carrier>,
    ) -> Option<Correlation> {
        let declared = names.get(raw)?;
        if !self.trusted.iter().any(|trusted| trusted.matches(sender)) {
            return Some(Correlation::Local {
                reason: InvalidCarrierReason::UntrustedWebview,
            });
        }
        let carrier = match carrier {
            Some(value) => value,
            None => {
                return Some(Correlation::Local {
                    reason: InvalidCarrierReason::Missing,
                })
            }
        };
        let reason = validate(carrier, declared, &self.baggage_allowlist);
        Some(match reason {
            Some(reason) => Correlation::Local { reason },
            None => Correlation::Remote(carrier.clone()),
        })
    }
}

fn validate(
    carrier: &Carrier,
    declared: &str,
    baggage_allowlist: &BTreeSet<String>,
) -> Option<InvalidCarrierReason> {
    if carrier.version != CARRIER_VERSION {
        return Some(InvalidCarrierReason::Version);
    }
    if !traceparent(&carrier.traceparent) {
        return Some(InvalidCarrierReason::Traceparent);
    }
    if carrier.semantic_name != declared {
        return Some(InvalidCarrierReason::SemanticMismatch);
    }
    if !semantic_name(&carrier.semantic_name) {
        return Some(InvalidCarrierReason::SemanticName);
    }
    if carrier.baggage.len() > MAX_BAGGAGE_ENTRIES
        || carrier.baggage.iter().any(|(key, value)| {
            key.len() > MAX_BAGGAGE_KEY_BYTES
                || value.len() > MAX_BAGGAGE_VALUE_BYTES
                || !token(key)
                || !baggage_allowlist.contains(key)
        })
    {
        return Some(InvalidCarrierReason::Baggage);
    }
    None
}

fn semantic_name(value: &str) -> bool {
    let mut previous_was_separator = false;
    !value.is_empty()
        && value.len() <= MAX_SEMANTIC_NAME_BYTES
        && value.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
        && value.bytes().all(|c| {
            let separator = matches!(c, b'.' | b'_' | b'-');
            let valid = (c.is_ascii_lowercase() || c.is_ascii_digit() || separator)
                && !(separator && previous_was_separator);
            previous_was_separator = separator;
            valid
        })
        && !previous_was_separator
}
fn token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}
fn traceparent(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    parts.len() == 4
        && parts[0] == "00"
        && parts[1].len() == 32
        && parts[2].len() == 16
        && parts[3].len() == 2
        && parts.iter().skip(1).all(|part| {
            part.bytes()
                .all(|c| c.is_ascii_digit() || matches!(c, b'a'..=b'f'))
        })
        && !parts[1].bytes().all(|c| c == b'0')
        && !parts[2].bytes().all(|c| c == b'0')
        && matches!(parts[3], "00" | "01")
}

/// Runs an application handler without inspecting its arguments, result, or error.
/// Call the public Chill Rust SDK around this boundary to record the resulting
/// semantic activity and bounded diagnostic reason.
pub fn without_payload_access<T, F: FnOnce() -> T>(_correlation: Correlation, handler: F) -> T {
    handler()
}

/// Host bridge using the public portable Rust SDK. It records only the declared
/// semantic activity and fixed diagnostic class, then invokes the supplied
/// future unchanged. Valid renderer metadata becomes an opaque remote trace
/// parent through the Rust SDK's public remote-context seam.
#[cfg(feature = "sdk")]
pub async fn activity<T, E, F>(
    client: &chill_observability::Client,
    semantic_name: &str,
    correlation: Correlation,
    handler: F,
) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    use chill_observability::{AnnotationValue, RemoteContext, SemanticName};
    use std::collections::BTreeMap;

    match correlation {
        Correlation::Local { reason } => {
            // A fixed semantic name is the bounded diagnostic. No application
            // errors or carrier content are ever passed to the Rust SDK.
            let diagnostic = format!("desktop.ipc.carrier_{}", reason.code());
            if let Ok(name) = SemanticName::try_from(diagnostic.as_str()) {
                client.event(name, BTreeMap::<_, AnnotationValue>::new());
            }
            let name = SemanticName::try_from(semantic_name)
                .expect("Correlator semantic names must be valid portable SDK names");
            client
                .activity(name, BTreeMap::<_, AnnotationValue>::new(), handler)
                .await
        }
        Correlation::Remote(carrier) => {
            // `Carrier` has already passed strict version, trust, semantic, and
            // bounded baggage validation. Only traceparent enters the SDK.
            let remote = RemoteContext::from_traceparent(&carrier.traceparent)
                .expect("validated carrier traceparent must create remote context");
            let name = SemanticName::try_from(semantic_name)
                .expect("Correlator semantic names must be valid portable SDK names");
            client
                .activity_with_context(
                    &remote,
                    name,
                    BTreeMap::<_, AnnotationValue>::new(),
                    handler,
                )
                .await
        }
    }
}

#[cfg(feature = "tauri")]
pub mod tauri2 {
    //! Tauri 2 command argument support. This reads only command configuration,
    //! request headers, and webview identity; it never calls `payload`, `body`,
    //! or formats application data.
    use super::*;
    use tauri::{
        ipc::{CommandArg, CommandItem, InvokeError},
        Runtime,
    };

    #[derive(Clone, Debug)]
    pub struct ChillIpcContext {
        pub semantic_name: Option<String>,
        pub correlation: Option<Correlation>,
    }

    impl ChillIpcContext {
        pub fn from_message<R: Runtime>(
            correlator: &Correlator,
            command: &str,
            message: &tauri::ipc::InvokeMessage<R>,
        ) -> Self {
            let webview = message.webview();
            let origin = webview
                .url()
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_default();
            let sender = WebviewIdentity {
                origin,
                label: webview.label().to_owned(),
            };
            let carrier = message
                .headers()
                .get("x-chill-carrier")
                .and_then(|header| header.to_str().ok())
                .and_then(parse_header);
            match correlator.command_context(command, &sender, carrier.as_ref()) {
                Some((semantic_name, correlation)) => Self {
                    semantic_name: Some(semantic_name.to_owned()),
                    correlation: Some(correlation),
                },
                None => Self {
                    semantic_name: None,
                    correlation: None,
                },
            }
        }
    }

    impl<'de, R: Runtime> CommandArg<'de, R> for ChillIpcContext {
        fn from_command(_command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
            // State access is not available to CommandArg; use `from_message`
            // with the app-owned correlator at the registration boundary.
            Ok(Self {
                semantic_name: None,
                correlation: None,
            })
        }
    }

    fn parse_header(value: &str) -> Option<Carrier> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct WireCarrier {
            version: u8,
            traceparent: String,
            #[serde(default)]
            baggage: BTreeMap<String, String>,
            semantic_name: String,
        }
        let wire: WireCarrier = serde_json::from_str(value).ok()?;
        Some(Carrier {
            version: wire.version,
            traceparent: wire.traceparent,
            baggage: wire.baggage,
            semantic_name: wire.semantic_name,
        })
    }
    pub use tauri;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sender() -> WebviewIdentity {
        WebviewIdentity {
            origin: "https://app.example.test".into(),
            label: "main".into(),
        }
    }
    fn carrier() -> Carrier {
        Carrier {
            version: 1,
            traceparent: "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01".into(),
            baggage: BTreeMap::new(),
            semantic_name: "account.save".into(),
        }
    }
    fn core() -> Correlator {
        Correlator::new([TrustedWebview {
            origin: "https://app.example.test".into(),
            label: "main".into(),
        }])
        .command("save_account", "account.save")
        .event("account_changed", "account.changed")
    }
    #[test]
    fn correlates_only_declared_command() {
        assert_eq!(
            core().command_correlation("save_account", &sender(), Some(&carrier())),
            Some(Correlation::Remote(carrier()))
        );
        assert_eq!(
            core().command_correlation("other", &sender(), Some(&carrier())),
            None
        );
    }
    #[test]
    fn events_use_their_declared_semantic_mapping() {
        let mut event_carrier = carrier();
        event_carrier.semantic_name = "account.changed".into();
        assert_eq!(
            core().event_correlation("account_changed", &sender(), Some(&event_carrier)),
            Some(Correlation::Remote(event_carrier))
        );
    }
    #[test]
    fn baggage_is_default_empty_and_explicitly_allowlisted() {
        let mut with_baggage = carrier();
        with_baggage
            .baggage
            .insert("public.region".into(), "us".into());
        assert_eq!(
            core().command_correlation("save_account", &sender(), Some(&with_baggage)),
            Some(Correlation::Local {
                reason: InvalidCarrierReason::Baggage
            })
        );
        assert_eq!(
            core()
                .allow_baggage_key("public.region")
                .command_correlation("save_account", &sender(), Some(&with_baggage)),
            Some(Correlation::Remote(with_baggage))
        );
    }
    #[test]
    fn semantic_first_character_and_trace_flags_are_strict() {
        assert!(std::panic::catch_unwind(|| core().command("bad", "bad..name")).is_err());
        let mut invalid = carrier();
        invalid.semantic_name = "-account.save".into();
        assert_eq!(
            core().command_correlation("save_account", &sender(), Some(&invalid)),
            Some(Correlation::Local {
                reason: InvalidCarrierReason::SemanticMismatch
            })
        );
        let mut invalid = carrier();
        invalid.traceparent = "00-0123456789ABCDEF0123456789ABCDEF-0123456789abcdef-01".into();
        assert_eq!(
            core().command_correlation("save_account", &sender(), Some(&invalid)),
            Some(Correlation::Local {
                reason: InvalidCarrierReason::Traceparent
            })
        );
        let mut invalid = carrier();
        invalid.traceparent = "00-0123456789abcdef0123456789abcdef-0123456789abcdef-02".into();
        assert_eq!(
            core().command_correlation("save_account", &sender(), Some(&invalid)),
            Some(Correlation::Local {
                reason: InvalidCarrierReason::Traceparent
            })
        );
    }
    #[test]
    fn exact_origin_and_label_are_required() {
        let mut bad = sender();
        bad.label = "other".into();
        assert_eq!(
            core().command_correlation("save_account", &bad, Some(&carrier())),
            Some(Correlation::Local {
                reason: InvalidCarrierReason::UntrustedWebview
            })
        );
    }
    #[test]
    fn invalid_carrier_starts_local_with_bounded_reason() {
        let mut invalid = carrier();
        invalid.traceparent = "secret error text".into();
        assert_eq!(
            core().command_correlation("save_account", &sender(), Some(&invalid)),
            Some(Correlation::Local {
                reason: InvalidCarrierReason::Traceparent
            })
        );
    }
    #[test]
    fn payload_stays_opaque() {
        let result = without_payload_access(
            Correlation::Local {
                reason: InvalidCarrierReason::Missing,
            },
            || String::from("application payload"),
        );
        assert_eq!(result, "application payload");
    }
}
