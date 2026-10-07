use serde::{Deserialize, Serialize};

use super::{ActorReference, CategoryPolicy, ReferencedResourceConfig};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advanced_security: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_security: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependabot_alerts: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependabot_security_updates: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_push_protection: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_validity_checks: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_non_provider_patterns: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_ai_detection: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_alert_dismissal: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_bypass: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_alert_dismissal_options: Option<SecurityReviewerOptionsConfigV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_bypass_options: Option<SecurityReviewerOptionsConfigV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_vulnerability_reporting: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codeql_default_setup: Option<CodeqlDefaultSetupConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_reference: Option<ReferencedResourceConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegated_alert_dismissal_reviewers: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegated_bypass_reviewers: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferencedResourceConfig>,
}

impl SecurityCategoryV2 {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            advanced_security: None,
            code_security: None,
            dependabot_alerts: None,
            dependabot_security_updates: None,
            secret_scanning: None,
            secret_scanning_push_protection: None,
            secret_scanning_validity_checks: None,
            secret_scanning_non_provider_patterns: None,
            secret_scanning_ai_detection: None,
            secret_scanning_delegated_alert_dismissal: None,
            secret_scanning_delegated_bypass: None,
            secret_scanning_delegated_alert_dismissal_options: None,
            secret_scanning_delegated_bypass_options: None,
            private_vulnerability_reporting: None,
            codeql_default_setup: None,
            configuration_reference: None,
            delegated_alert_dismissal_reviewers: Vec::new(),
            delegated_bypass_reviewers: Vec::new(),
            references: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityReviewerOptionsConfigV2 {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<SecurityReviewerConfigV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityReviewerConfigV2 {
    pub actor: ActorReference,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodeqlDefaultSetupConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_suite: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_label: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat_model: Option<String>,
}
