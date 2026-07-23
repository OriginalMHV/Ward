use serde::{Deserialize, Serialize};

use super::v2::{CoverageEntry, ManifestCategories, ManifestProvenance, ManifestSchema};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub org: OrgConfig,

    #[serde(default)]
    pub file_delivery: FileDeliveryConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<SystemConfig>,

    #[serde(default = "ManifestSchema::current")]
    pub schema: ManifestSchema,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ManifestProvenance>,

    #[serde(default, skip_serializing_if = "ManifestCategories::is_empty")]
    pub categories: ManifestCategories,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OrgConfig {
    pub name: String,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct RepositorySettingsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_issues: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_projects: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_wiki: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_discussions: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_pull_requests: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request_creation_policy: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_sponsorships_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_creation_policy: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_squash_merge: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_merge_commit: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_rebase_merge: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_auto_merge: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_branch_on_merge: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_update_branch: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squash_merge_commit_title: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squash_merge_commit_message: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_commit_title: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_commit_message: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_commit_signoff_required: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_squash_pr_title_as_default: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<String>>,
}

/// Pull request delivery settings for Ward-managed file synchronization:
/// the branch Ward pushes to, the reviewers it requests, and the commit
/// message prefix it uses.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FileDeliveryConfig {
    #[serde(default = "default_branch_name")]
    pub branch: String,

    #[serde(default)]
    pub reviewers: Vec<String>,

    #[serde(default = "default_commit_prefix")]
    pub commit_message_prefix: String,
}

impl Default for FileDeliveryConfig {
    fn default() -> Self {
        Self {
            branch: default_branch_name(),
            reviewers: Vec::new(),
            commit_message_prefix: default_commit_prefix(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct BranchProtectionConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_one")]
    pub required_approvals: u32,

    #[serde(default)]
    pub dismiss_stale_reviews: bool,

    #[serde(default)]
    pub require_code_owner_reviews: bool,

    #[serde(default)]
    pub require_status_checks: bool,

    #[serde(default)]
    pub strict_status_checks: bool,

    #[serde(default)]
    pub enforce_admins: bool,

    #[serde(default)]
    pub required_linear_history: bool,

    #[serde(default)]
    pub allow_force_pushes: bool,

    #[serde(default)]
    pub allow_deletions: bool,
}

fn default_one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RepositoryRuleConfig {
    #[serde(rename = "type")]
    pub rule_type: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters_json: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct TeamAccess {
    pub slug: String,
    pub permission: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SystemConfig {
    pub id: String,
    pub name: String,

    #[serde(default = "default_true")]
    pub match_prefix: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,

    /// Complete category overrides for repositories selected through this
    /// system. A configured category replaces the global category for those
    /// repositories; omitted categories inherit the global desired state.
    #[serde(default, skip_serializing_if = "ManifestCategories::is_empty")]
    pub categories: ManifestCategories,
}

pub(crate) fn default_true() -> bool {
    true
}

fn default_branch_name() -> String {
    "chore/ward-sync".to_owned()
}

fn default_commit_prefix() -> String {
    "chore: ".to_owned()
}
