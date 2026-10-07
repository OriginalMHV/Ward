use serde::{Deserialize, Serialize};

use super::{CategoryPolicy, NamedValueConfig, ReferencedResourceConfig, SecretPlaceholderConfig};

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionsCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<ActionsSettingsConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<NamedValueConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretPlaceholderConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependabot_secrets: Vec<SecretPlaceholderConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codespaces_secrets: Vec<SecretPlaceholderConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<WorkflowStateConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferencedResourceConfig>,
}

impl ActionsCategoryV2 {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionsSettingsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_actions: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_actions: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_github_owned_actions: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_verified_creator_actions: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_pinned_actions: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workflow_permissions: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_approve_pull_request_reviews: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_retention_days: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_retention_days: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_fork_workflows_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_fork_workflow_approval: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_write_tokens_to_workflows: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_secrets_and_variables: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_approval_for_fork_pr_workflows: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_pull_request_workflows_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_pull_request_contributor_approval: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_access_level: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_subject_claim_template: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_use_default: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_use_immutable_subject: Option<bool>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oidc_subject_claim_include_keys: Vec<String>,

    /// `GET/PUT /repos/{owner}/{repo}/actions/cache/retention-limit`'s
    /// `max_cache_retention_days`. Distinct from `artifact_retention_days`/
    /// `log_retention_days` (a different, older endpoint) — this limits how
    /// long GitHub Actions *dependency caches* (`actions/cache`) may be
    /// retained before eviction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_retention_limit_days: Option<u32>,

    /// `GET/PUT /repos/{owner}/{repo}/actions/cache/storage-limit`'s
    /// `max_cache_size_gb`. This is a writable policy limit, not the
    /// current cache usage (`GET .../actions/cache/usage`, which is
    /// runtime data and is never part of desired configuration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_storage_limit_gb: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowStateConfig {
    pub path: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}
