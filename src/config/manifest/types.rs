use serde::{Deserialize, Serialize};

use super::v2::ManifestV2State;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Manifest {
    pub org: OrgConfig,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceConfig>,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<RepositorySettingsConfig>,

    #[serde(default)]
    pub file_delivery: FileDeliveryConfig,

    #[serde(default)]
    pub branch_protection: BranchProtectionConfig,

    #[serde(default)]
    pub rulesets: RulesetsConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<SystemConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ManagedFile>,

    #[serde(flatten)]
    pub v2: ManifestV2State,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OrgConfig {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SourceConfig {
    pub repository: String,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub secret_scanning: bool,

    #[serde(default = "default_true")]
    pub secret_scanning_ai_detection: bool,

    #[serde(default = "default_true")]
    pub push_protection: bool,

    #[serde(default = "default_true")]
    pub dependabot_alerts: bool,

    #[serde(default = "default_true")]
    pub dependabot_security_updates: bool,

    #[serde(default)]
    pub codeql_advanced_setup: bool,
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
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct FileDeliveryConfig {
    #[serde(default = "default_branch_name")]
    pub branch: String,

    #[serde(default)]
    pub reviewers: Vec<String>,

    #[serde(default = "default_commit_prefix")]
    pub commit_message_prefix: String,
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

fn default_active() -> String {
    "active".to_string()
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct RulesetsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_protection: Option<RulesetBranchProtection>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repository: Vec<RepositoryRulesetConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RulesetBranchProtection {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default)]
    pub name: Option<String>,

    #[serde(default = "default_active")]
    pub enforcement: String,

    #[serde(default = "default_one")]
    pub required_approvals: u32,

    #[serde(default)]
    pub dismiss_stale_reviews: bool,

    #[serde(default)]
    pub require_code_owner_reviews: bool,

    #[serde(default)]
    pub required_status_checks: Vec<String>,

    #[serde(default)]
    pub require_linear_history: bool,

    #[serde(default)]
    pub block_force_pushes: bool,

    #[serde(default)]
    pub block_deletions: bool,

    #[serde(default)]
    pub bypass_teams: Vec<BypassTeam>,

    #[serde(default)]
    pub overrides: Vec<RepoOverride>,
}

/// A bypass team entry that supports both simple string and detailed forms.
///
/// Simple form (defaults to bypass_mode = "always"):
/// ```toml
/// bypass_teams = ["my-team"]
/// ```
///
/// Detailed form with explicit bypass_mode:
/// ```toml
/// bypass_teams = [{ slug = "my-team", bypass_mode = "pull_request" }]
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum BypassTeam {
    /// Simple form: just a team slug string (defaults to bypass_mode = "always")
    Simple(String),
    /// Detailed form: team slug with explicit bypass_mode
    Detailed {
        slug: String,
        #[serde(default = "default_bypass_mode")]
        bypass_mode: String,
    },
}

fn default_bypass_mode() -> String {
    "always".to_string()
}

impl BypassTeam {
    pub fn slug(&self) -> &str {
        match self {
            BypassTeam::Simple(s) => s,
            BypassTeam::Detailed { slug, .. } => slug,
        }
    }

    pub fn bypass_mode(&self) -> &str {
        match self {
            BypassTeam::Simple(_) => "always",
            BypassTeam::Detailed { bypass_mode, .. } => bypass_mode,
        }
    }
}

/// Per-repo overrides within a system or global config.
/// Repos matching any of the glob patterns get different ruleset settings.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RepoOverride {
    /// Glob patterns matching repo names (e.g., ["*-operations", "*-system"])
    pub repo_patterns: Vec<String>,

    /// Override fields for matching repos
    #[serde(flatten)]
    pub overrides: RulesetBranchProtectionOverride,
}

/// Per-system override for ruleset branch protection.
/// All fields are optional; only explicitly set fields override the global config.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct RulesetBranchProtectionOverride {
    pub enabled: Option<bool>,
    pub name: Option<String>,
    pub enforcement: Option<String>,
    pub required_approvals: Option<u32>,
    pub dismiss_stale_reviews: Option<bool>,
    pub require_code_owner_reviews: Option<bool>,
    pub required_status_checks: Option<Vec<String>>,
    pub require_linear_history: Option<bool>,
    pub block_force_pushes: Option<bool>,
    pub block_deletions: Option<bool>,
    pub bypass_teams: Option<Vec<BypassTeam>>,
    pub overrides: Option<Vec<RepoOverride>>,
}

/// Per-system rulesets override container.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct RulesetsOverrideConfig {
    #[serde(default)]
    pub branch_protection: Option<RulesetBranchProtectionOverride>,
}

impl RulesetBranchProtection {
    /// Merge this config with a per-system override.
    /// Override fields take precedence; unset fields fall back to self.
    pub fn merge_with(&self, over: &RulesetBranchProtectionOverride) -> Self {
        Self {
            enabled: over.enabled.unwrap_or(self.enabled),
            name: over.name.clone().or_else(|| self.name.clone()),
            enforcement: over
                .enforcement
                .clone()
                .unwrap_or_else(|| self.enforcement.clone()),
            required_approvals: over.required_approvals.unwrap_or(self.required_approvals),
            dismiss_stale_reviews: over
                .dismiss_stale_reviews
                .unwrap_or(self.dismiss_stale_reviews),
            require_code_owner_reviews: over
                .require_code_owner_reviews
                .unwrap_or(self.require_code_owner_reviews),
            required_status_checks: over
                .required_status_checks
                .clone()
                .unwrap_or_else(|| self.required_status_checks.clone()),
            require_linear_history: over
                .require_linear_history
                .unwrap_or(self.require_linear_history),
            block_force_pushes: over.block_force_pushes.unwrap_or(self.block_force_pushes),
            block_deletions: over.block_deletions.unwrap_or(self.block_deletions),
            bypass_teams: over
                .bypass_teams
                .clone()
                .unwrap_or_else(|| self.bypass_teams.clone()),
            overrides: over
                .overrides
                .clone()
                .unwrap_or_else(|| self.overrides.clone()),
        }
    }

    /// Returns the final config for a specific repo, applying any matching repo pattern overrides.
    /// The first matching override wins. The returned config has an empty overrides vec
    /// to prevent recursive override application.
    pub fn for_repo(&self, repo_name: &str) -> Self {
        let matching_override = self.overrides.iter().find(|o| {
            o.repo_patterns
                .iter()
                .any(|pattern| glob_match::glob_match(pattern, repo_name))
        });

        match matching_override {
            Some(o) => {
                let mut resolved = self.merge_with(&o.overrides);
                resolved.overrides = Vec::new();
                resolved
            }
            None => {
                let mut base = self.clone();
                base.overrides = Vec::new();
                base
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RepositoryRulesetConfig {
    pub name: String,
    pub target: String,
    pub enforcement: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions_json: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RepositoryRuleConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bypass_actors: Vec<RulesetBypassActorConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RepositoryRuleConfig {
    #[serde(rename = "type")]
    pub rule_type: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RulesetBypassActorConfig {
    pub actor_type: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_slug: Option<String>,

    #[serde(default = "default_bypass_mode")]
    pub bypass_mode: String,
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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<SecurityConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<TeamAccess>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rulesets: Option<RulesetsOverrideConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ManagedFile {
    pub path: String,
    pub content: String,
}

pub(crate) fn default_true() -> bool {
    true
}

fn default_branch_name() -> String {
    "chore/ward-setup".to_owned()
}

fn default_commit_prefix() -> String {
    "chore: ".to_owned()
}
