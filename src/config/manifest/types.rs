use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::cli::policy::PolicyRule;

#[derive(Debug, PartialEq, Deserialize)]
pub struct Manifest {
    pub org: OrgConfig,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default)]
    pub templates: TemplateConfig,

    #[serde(default)]
    pub branch_protection: BranchProtectionConfig,

    #[serde(default)]
    pub rulesets: RulesetsConfig,

    #[serde(default)]
    pub systems: Vec<SystemConfig>,

    #[serde(default)]
    pub policies: Vec<PolicyRule>,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct OrgConfig {
    pub name: String,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
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

    /// Custom security checks shown as extra columns in the TUI security tab.
    #[serde(default)]
    pub checks: Vec<SecurityCheck>,
}

/// A user-defined check rendered as an extra column in the TUI security tab.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecurityCheck {
    FileExists { name: String, path: String },
    WorkflowExists { name: String, path: String },
    TopicContains { name: String, topic: String },
    BranchProtection { name: String },
    DefaultBranch { name: String, expected: String },
}

impl SecurityCheck {
    /// Display name used as the column header in the TUI.
    pub fn name(&self) -> &str {
        match self {
            Self::FileExists { name, .. }
            | Self::WorkflowExists { name, .. }
            | Self::TopicContains { name, .. }
            | Self::BranchProtection { name }
            | Self::DefaultBranch { name, .. } => name,
        }
    }
}

#[derive(Debug, Default, PartialEq, Deserialize)]
pub struct TemplateConfig {
    #[serde(default = "default_branch_name")]
    pub branch: String,

    #[serde(default)]
    pub reviewers: Vec<String>,

    #[serde(default = "default_commit_prefix")]
    pub commit_message_prefix: String,

    #[serde(default)]
    pub custom_dir: Option<String>,

    #[serde(default)]
    pub registries: HashMap<String, RegistryConfig>,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct RulesetsConfig {
    #[serde(default)]
    pub branch_protection: Option<RulesetBranchProtection>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RepoOverride {
    /// Glob patterns matching repo names (e.g., ["*-operations", "*-system"])
    pub repo_patterns: Vec<String>,

    /// Override fields for matching repos
    #[serde(flatten)]
    pub overrides: RulesetBranchProtectionOverride,
}

/// Per-system override for ruleset branch protection.
/// All fields are optional; only explicitly set fields override the global config.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
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
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct TeamAccess {
    pub slug: String,
    pub permission: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RegistryConfig {
    #[serde(rename = "type")]
    pub registry_type: String,
    pub url: String,
    #[serde(default)]
    pub jfrog_oidc_provider: Option<String>,
    #[serde(default)]
    pub audience: Option<String>,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct SystemConfig {
    pub id: String,
    pub name: String,

    #[serde(default)]
    pub exclude: Vec<String>,

    #[serde(default)]
    pub repos: Vec<String>,

    #[serde(default)]
    pub security: Option<SecurityConfig>,

    #[serde(default)]
    pub teams: Vec<TeamAccess>,

    #[serde(default)]
    pub rulesets: Option<RulesetsOverrideConfig>,
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
