use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ControlPlaneError;

static POLICY_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
        .unwrap_or_else(|error| unreachable!("static policy-version regex is invalid: {error}"))
});
static ANNOTATION_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$")
        .unwrap_or_else(|error| unreachable!("static annotation-name regex is invalid: {error}"))
});

const CLASSIFICATIONS: &[&str] = &[
    "public",
    "internal",
    "pseudonymous_identifier",
    "personal_data",
    "sensitive_data",
    "secret",
    "credential",
];
const ELIGIBLE_CLASSIFICATIONS: &[&str] = &[
    "public",
    "internal",
    "pseudonymous_identifier",
    "personal_data",
];
const SOURCE_ALLOWLIST: &[&str] = &[
    "app_build",
    "app_version",
    "os_name",
    "os_version",
    "platform",
];

/// Fully validated, fail-closed environment privacy policy.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPrivacyPolicy {
    /// Policy schema version.
    pub schema_version: String,
    /// Operator-visible policy version.
    pub policy_version: String,
    /// Default disposition, required to be `omit`.
    pub default_disposition: String,
    /// Explicit annotation classifications.
    pub annotation_allowlist: HashMap<String, String>,
    /// Explicit safe source fields.
    pub source_allowlist: Vec<String>,
    /// Source-side replay masking contract.
    pub replay: ReplayPolicy,
}

/// Privacy requirements for structural replay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayPolicy {
    /// Whether masking occurs before leaving the client.
    pub mask_at_source: bool,
    /// Text handling.
    pub text: String,
    /// Form-value handling.
    pub form_values: String,
    /// Accessibility-text handling.
    pub accessibility_text: String,
    /// Secure-input handling.
    pub secure_input: String,
    /// Pixel handling.
    pub pixels: String,
    /// Custom drawing handling.
    pub custom_drawing: String,
}

/// Parses and validates the complete privacy policy without permissive defaults.
///
/// # Errors
///
/// Returns [`ControlPlaneError::InvalidInput`] for unknown fields, versions,
/// classifications, sources, duplicates, or unsafe replay behavior.
pub fn parse_environment_privacy_policy(
    value: Value,
) -> Result<EnvironmentPrivacyPolicy, ControlPlaneError> {
    let policy: EnvironmentPrivacyPolicy = serde_json::from_value(value).map_err(|_| {
        ControlPlaneError::InvalidInput("environment privacy policy is invalid".to_owned())
    })?;
    if policy.schema_version != "1.0.0" || !POLICY_VERSION.is_match(&policy.policy_version) {
        return Err(ControlPlaneError::InvalidInput(
            "environment privacy policy version is invalid".to_owned(),
        ));
    }
    if policy.default_disposition != "omit" {
        return Err(ControlPlaneError::InvalidInput(
            "environment privacy policy must default to omit".to_owned(),
        ));
    }
    if policy.annotation_allowlist.len() > 128 {
        return Err(ControlPlaneError::InvalidInput(
            "environment privacy annotation allowlist is invalid".to_owned(),
        ));
    }
    for (name, classification) in &policy.annotation_allowlist {
        if name.len() > 128
            || !ANNOTATION_NAME.is_match(name)
            || !CLASSIFICATIONS.contains(&classification.as_str())
        {
            return Err(ControlPlaneError::InvalidInput(
                "environment privacy annotation is invalid".to_owned(),
            ));
        }
    }
    if policy.source_allowlist.len() > 64 {
        return Err(ControlPlaneError::InvalidInput(
            "environment source allowlist is too large".to_owned(),
        ));
    }
    let mut seen = HashSet::new();
    for source in &policy.source_allowlist {
        if !SOURCE_ALLOWLIST.contains(&source.as_str()) || !seen.insert(source) {
            return Err(ControlPlaneError::InvalidInput(
                "environment privacy source is invalid".to_owned(),
            ));
        }
    }
    let replay = &policy.replay;
    if !replay.mask_at_source
        || replay.text != "mask"
        || replay.form_values != "mask"
        || replay.accessibility_text != "mask"
        || replay.secure_input != "mask"
        || replay.pixels != "mask"
        || replay.custom_drawing != "block"
    {
        return Err(ControlPlaneError::InvalidInput(
            "environment replay policy must mask at source and block custom drawing".to_owned(),
        ));
    }
    Ok(policy)
}

/// Tests whether a classification may pass the annotation allowlist.
#[must_use]
pub fn annotation_classification_is_eligible(classification: &str) -> bool {
    ELIGIBLE_CLASSIFICATIONS.contains(&classification)
}

pub(crate) fn policy_version_is_valid(version: &str) -> bool {
    POLICY_VERSION.is_match(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_policy_fails_closed() {
        let valid = serde_json::json!({
            "schema_version":"1.0.0", "policy_version":"privacy-v1",
            "default_disposition":"omit", "annotation_allowlist":{"cat.id":"pseudonymous_identifier"},
            "source_allowlist":["platform"], "replay":{"mask_at_source":true,"text":"mask",
            "form_values":"mask","accessibility_text":"mask","secure_input":"mask",
            "pixels":"mask","custom_drawing":"block"}
        });
        assert!(parse_environment_privacy_policy(valid.clone()).is_ok());
        let mut unsafe_policy = valid;
        unsafe_policy["replay"]["mask_at_source"] = Value::Bool(false);
        assert!(parse_environment_privacy_policy(unsafe_policy).is_err());
        assert!(annotation_classification_is_eligible("personal_data"));
        assert!(!annotation_classification_is_eligible("credential"));
    }
}
