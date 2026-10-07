use serde::{Deserialize, Serialize};

use super::{
    ActorReference, CategoryPolicy, NamedValueConfig, ReferencedResourceConfig,
    SecretPlaceholderConfig,
};

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentsCategory {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<EnvironmentConfig>,
}

impl EnvironmentsCategory {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentConfig {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_timer_minutes: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prevent_self_review: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_policy: Option<EnvironmentDeploymentPolicyConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<EnvironmentReviewerConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protection_apps: Vec<ReferencedResourceConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<NamedValueConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretPlaceholderConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDeploymentPolicyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_branches: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_branch_policies: Option<bool>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branch_patterns: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentReviewerConfig {
    pub actor: ActorReference,
}
