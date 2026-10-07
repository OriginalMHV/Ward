use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ActorReference, CategoryPolicy};
use crate::config::manifest::BranchProtectionConfig;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchProtectionCategory {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<BranchProtectionConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch_detailed: Option<DetailedBranchProtectionConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_branches: Vec<ProtectedBranchConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DetailedBranchProtectionConfig {
    #[serde(default)]
    pub protection: BranchProtectionConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_check_contexts: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_checks: Vec<BranchStatusCheckConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub push_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissal_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_request_bypass_allowances: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_last_push_approval: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_creations: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_reviewers: Option<Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_conversation_resolution: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_signed_commits: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_branch: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fork_syncing: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedBranchConfig {
    pub name: String,

    #[serde(default)]
    pub protection: BranchProtectionConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_check_contexts: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_checks: Vec<BranchStatusCheckConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub push_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissal_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_request_bypass_allowances: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_last_push_approval: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_creations: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_reviewers: Option<Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_conversation_resolution: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_signed_commits: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_branch: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fork_syncing: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchStatusCheckConfig {
    pub context: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_slug: Option<String>,
}
