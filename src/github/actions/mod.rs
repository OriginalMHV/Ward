//! Repository GitHub Actions APIs.
//!
//! Endpoints implemented here are verified against the GitHub REST API
//! reference (`X-GitHub-Api-Version: 2022-11-28`) for:
//! `actions/permissions`, `actions/workflows`, `actions/variables`,
//! `actions/secrets`, `actions/oidc`, `dependabot/secrets`, and
//! `codespaces/repository-secrets`.

use serde::{Deserialize, Serialize};

mod permissions;
mod secrets;
mod variables;
mod workflows;

pub use crate::github::outcome::{ReadOutcome, WriteOutcome};
pub(crate) use crate::github::outcome::{classify_read, write_delete, write_empty, write_json};
pub use secrets::seal_secret_value;

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ActionsPermissions {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_actions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha_pinning_required: Option<bool>,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/selected-actions`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct SelectedActionsPolicy {
    #[serde(default)]
    pub github_owned_allowed: bool,
    #[serde(default)]
    pub verified_allowed: bool,
    #[serde(default)]
    pub patterns_allowed: Vec<String>,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/workflow`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorkflowPermissions {
    pub default_workflow_permissions: String,
    pub can_approve_pull_request_reviews: bool,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/artifact-and-log-retention`.
///
/// GitHub exposes a single combined retention setting for both artifacts and
/// logs; there is no separate REST control for the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ArtifactLogRetention {
    pub days: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_allowed_days: Option<u32>,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/cache/retention-limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ActionsCacheRetentionLimit {
    pub max_cache_retention_days: u32,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/cache/storage-limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ActionsCacheStorageLimit {
    pub max_cache_size_gb: u32,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/fork-pr-contributor-approval`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ForkPrContributorApproval {
    pub approval_policy: String,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/fork-pr-workflows-private-repos`.
///
/// This endpoint only applies to private and internal repositories; GitHub
/// returns 404 for public repositories, which callers should treat as
/// "not applicable" rather than an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct PrivateForkPrWorkflows {
    pub run_workflows_from_fork_pull_requests: bool,
    #[serde(default)]
    pub send_write_tokens_to_workflows: bool,
    #[serde(default)]
    pub send_secrets_and_variables: bool,
    #[serde(default)]
    pub require_approval_for_fork_pr_workflows: bool,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/access`.
///
/// This endpoint only applies to private repositories.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WorkflowAccessLevel {
    pub access_level: String,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/oidc/customization/sub`.
///
/// `sub_claim_prefix` is computed by GitHub and is not a settable body
/// parameter on the PUT endpoint, so it is observed but never written back.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct OidcSubjectClaim {
    #[serde(default)]
    pub use_default: bool,
    #[serde(default)]
    pub include_claim_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_immutable_subject: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_claim_prefix: Option<String>,
}

/// A single workflow, as returned by `GET /repos/{owner}/{repo}/actions/workflows`.
#[derive(Debug, Clone, Deserialize)]
pub struct Workflow {
    pub id: u64,
    pub path: String,
    pub state: String,
}

/// A self-hosted runner label, as returned inline on a runner.
#[derive(Debug, Clone, Deserialize)]
pub struct SelfHostedRunnerLabel {
    pub name: String,
}

/// A self-hosted runner, as returned by
/// `GET /repos/{owner}/{repo}/actions/runners`. Ward only ever reads this
/// endpoint: it never registers, re-registers, or deletes runners. The
/// numeric `id`/`runner_group_id` fields are intentionally not modeled here
/// since they must never be persisted as manifest diagnostics (source IDs).
#[derive(Debug, Clone, Deserialize)]
pub struct SelfHostedRunner {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub labels: Vec<SelfHostedRunnerLabel>,
}

/// A repository or environment Actions variable.
#[derive(Debug, Clone, Deserialize)]
pub struct ActionsVariable {
    pub name: String,
    pub value: String,
}

/// Secret metadata as returned by GitHub. The encrypted value is never
/// returned by any GitHub API and must never be requested or logged.
#[derive(Debug, Clone, Deserialize)]
pub struct SecretMetadata {
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// The public key used to encrypt secret values for a repository or environment.
#[derive(Debug, Clone, Deserialize)]
pub struct SecretPublicKey {
    pub key_id: String,
    pub key: String,
}
