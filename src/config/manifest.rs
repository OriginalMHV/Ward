use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
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

impl Manifest {
    pub fn load(path: Option<&str>) -> Result<Self> {
        let default_path = "ward.toml";
        let path = path.unwrap_or(default_path);

        if !Path::new(path).exists() {
            tracing::info!("No ward.toml found, using defaults");
            return Ok(Self::default());
        }

        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {path}"))?;

        toml::from_str(&content).with_context(|| format!("Failed to parse {path}"))
    }

    pub fn system(&self, id: &str) -> Option<&SystemConfig> {
        self.systems.iter().find(|s| s.id == id)
    }

    pub fn security_for_system(&self, system_id: &str) -> &SecurityConfig {
        self.systems
            .iter()
            .find(|s| s.id == system_id)
            .and_then(|s| s.security.as_ref())
            .unwrap_or(&self.security)
    }

    /// Returns the merged rulesets branch protection config for a system.
    /// Per-system override fields take precedence; unset fields fall back to global.
    pub fn rulesets_branch_protection_for_system(
        &self,
        system_id: &str,
    ) -> Option<RulesetBranchProtection> {
        let global = self.rulesets.branch_protection.as_ref()?;

        let system_override = self
            .systems
            .iter()
            .find(|s| s.id == system_id)
            .and_then(|s| s.rulesets.as_ref())
            .and_then(|r| r.branch_protection.as_ref());

        match system_override {
            Some(over) => Some(global.merge_with(over)),
            None => Some(global.clone()),
        }
    }

    pub fn exclude_patterns_for_system(&self, system_id: &str) -> Vec<String> {
        self.systems
            .iter()
            .find(|s| s.id == system_id)
            .map(|s| s.exclude.clone())
            .unwrap_or_default()
    }

    pub fn explicit_repos_for_system(&self, system_id: &str) -> Vec<String> {
        self.systems
            .iter()
            .find(|s| s.id == system_id)
            .map(|s| s.repos.clone())
            .unwrap_or_default()
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            org: OrgConfig {
                name: String::new(),
            },
            security: SecurityConfig::default(),
            templates: TemplateConfig::default(),
            branch_protection: BranchProtectionConfig::default(),
            rulesets: RulesetsConfig::default(),
            systems: Vec::new(),
            policies: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_branch_name() -> String {
    "chore/ward-setup".to_owned()
}

fn default_commit_prefix() -> String {
    "chore: ".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
            [org]
            name = "test-org"
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.org.name, "test-org");
        // #[serde(default)] on the struct field uses derive(Default), not serde field defaults
        assert!(!m.security.secret_scanning);
        assert!(m.systems.is_empty());
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
            [org]
            name = "my-org"
            [security]
            secret_scanning = false
            push_protection = true
            dependabot_alerts = true
            dependabot_security_updates = false
            [templates]
            branch = "feat/setup"
            reviewers = ["alice"]
            [[systems]]
            id = "backend"
            name = "Backend"
            exclude = ["ops", "infra"]
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.org.name, "my-org");
        assert!(!m.security.secret_scanning);
        assert!(m.security.push_protection);
        assert!(!m.security.dependabot_security_updates);
        assert_eq!(m.templates.branch, "feat/setup");
        assert_eq!(m.templates.reviewers, vec!["alice"]);
        assert_eq!(m.systems.len(), 1);
        assert_eq!(m.systems[0].id, "backend");
        assert_eq!(m.systems[0].exclude, vec!["ops", "infra"]);
    }

    #[test]
    fn system_lookup() {
        let toml = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
            [[systems]]
            id = "fe"
            name = "Frontend"
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.system("be").unwrap().name, "Backend");
        assert_eq!(m.system("fe").unwrap().name, "Frontend");
        assert!(m.system("missing").is_none());
    }

    #[test]
    fn security_for_system_falls_back_to_global() {
        let toml = r#"
            [org]
            name = "org"
            [security]
            secret_scanning = false
            [[systems]]
            id = "be"
            name = "Backend"
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert!(!m.security_for_system("be").secret_scanning);
    }

    #[test]
    fn security_for_system_uses_override() {
        let toml = r#"
            [org]
            name = "org"
            [security]
            secret_scanning = true
            [[systems]]
            id = "be"
            name = "Backend"
            [systems.security]
            secret_scanning = false
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert!(!m.security_for_system("be").secret_scanning);
    }

    #[test]
    fn exclude_patterns_for_unknown_system_returns_empty() {
        let m = Manifest::default();
        assert!(m.exclude_patterns_for_system("nope").is_empty());
    }

    #[test]
    fn exclude_patterns_for_known_system() {
        let toml = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
            exclude = ["ops", "infra"]
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.exclude_patterns_for_system("be"), vec!["ops", "infra"]);
    }

    #[test]
    fn system_with_explicit_repos() {
        let toml = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
            repos = ["standalone-service", "legacy-api"]
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(
            m.explicit_repos_for_system("be"),
            vec!["standalone-service", "legacy-api"]
        );
    }

    #[test]
    fn system_without_explicit_repos_returns_empty() {
        let toml = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert!(m.explicit_repos_for_system("be").is_empty());
    }

    #[test]
    fn branch_protection_serde_defaults() {
        let bp: BranchProtectionConfig = toml::from_str("").unwrap();
        assert!(!bp.enabled);
        assert_eq!(bp.required_approvals, 1);
        assert!(!bp.dismiss_stale_reviews);
        assert!(!bp.require_code_owner_reviews);
        assert!(!bp.require_status_checks);
        assert!(!bp.strict_status_checks);
        assert!(!bp.enforce_admins);
        assert!(!bp.required_linear_history);
        assert!(!bp.allow_force_pushes);
        assert!(!bp.allow_deletions);
    }

    #[test]
    fn branch_protection_full_parse() {
        let toml_str = r#"
            enabled = true
            required_approvals = 2
            dismiss_stale_reviews = true
            require_code_owner_reviews = true
            enforce_admins = true
        "#;
        let bp: BranchProtectionConfig = toml::from_str(toml_str).unwrap();
        assert!(bp.enabled);
        assert_eq!(bp.required_approvals, 2);
        assert!(bp.dismiss_stale_reviews);
        assert!(bp.require_code_owner_reviews);
        assert!(bp.enforce_admins);
        assert!(!bp.allow_force_pushes);
    }

    #[test]
    fn default_template_config_values() {
        // derive(Default) gives empty strings/vecs, not the serde defaults
        let tc = TemplateConfig::default();
        assert_eq!(tc.branch, "");
        assert_eq!(tc.commit_message_prefix, "");
        assert!(tc.reviewers.is_empty());
        assert!(tc.registries.is_empty());
    }

    #[test]
    fn serde_template_config_defaults() {
        // When deserialized with missing fields, serde uses the custom defaults
        let tc: TemplateConfig = toml::from_str("").unwrap();
        assert_eq!(tc.branch, "chore/ward-setup");
        assert_eq!(tc.commit_message_prefix, "chore: ");
        assert!(tc.reviewers.is_empty());
        assert!(tc.registries.is_empty());
    }

    #[test]
    fn default_security_config_all_false() {
        // derive(Default) sets all bools to false
        let sc = SecurityConfig::default();
        assert!(!sc.secret_scanning);
        assert!(!sc.secret_scanning_ai_detection);
        assert!(!sc.push_protection);
        assert!(!sc.dependabot_alerts);
        assert!(!sc.dependabot_security_updates);
        assert!(!sc.codeql_advanced_setup);
    }

    #[test]
    fn serde_security_config_defaults_to_true() {
        // When deserialized with missing fields, serde uses default_true
        let sc: SecurityConfig = toml::from_str("").unwrap();
        assert!(sc.secret_scanning);
        assert!(sc.secret_scanning_ai_detection);
        assert!(sc.push_protection);
        assert!(sc.dependabot_alerts);
        assert!(sc.dependabot_security_updates);
        assert!(!sc.codeql_advanced_setup); // this one defaults false
    }

    #[test]
    fn rulesets_config_defaults() {
        let rc = RulesetsConfig::default();
        assert!(rc.branch_protection.is_none());
    }

    #[test]
    fn rulesets_config_serde_defaults() {
        let rc: RulesetsConfig = toml::from_str("").unwrap();
        assert!(rc.branch_protection.is_none());
    }

    #[test]
    fn ruleset_branch_protection_serde_defaults() {
        let rbp: RulesetBranchProtection = toml::from_str("").unwrap();
        assert!(rbp.enabled);
        assert!(rbp.name.is_none());
        assert_eq!(rbp.enforcement, "active");
        assert_eq!(rbp.required_approvals, 1);
        assert!(!rbp.dismiss_stale_reviews);
        assert!(!rbp.require_code_owner_reviews);
        assert!(rbp.required_status_checks.is_empty());
        assert!(!rbp.require_linear_history);
        assert!(!rbp.block_force_pushes);
        assert!(!rbp.block_deletions);
        assert!(rbp.bypass_teams.is_empty());
    }

    #[test]
    fn ruleset_branch_protection_custom_values() {
        let toml_str = r#"
            enabled = true
            name = "Custom Rules"
            enforcement = "evaluate"
            required_approvals = 2
            dismiss_stale_reviews = true
            require_code_owner_reviews = true
            required_status_checks = ["ci", "lint"]
            require_linear_history = true
            block_force_pushes = true
            block_deletions = true
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        assert!(rbp.enabled);
        assert_eq!(rbp.name.as_deref(), Some("Custom Rules"));
        assert_eq!(rbp.enforcement, "evaluate");
        assert_eq!(rbp.required_approvals, 2);
        assert!(rbp.dismiss_stale_reviews);
        assert!(rbp.require_code_owner_reviews);
        assert_eq!(rbp.required_status_checks, vec!["ci", "lint"]);
        assert!(rbp.require_linear_history);
        assert!(rbp.block_force_pushes);
        assert!(rbp.block_deletions);
    }

    #[test]
    fn team_access_empty_default() {
        let toml_str = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        assert!(m.systems[0].teams.is_empty());
    }

    #[test]
    fn team_access_parsing() {
        let toml_str = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
            teams = [
                { slug = "developers", permission = "push" },
                { slug = "devops", permission = "admin" },
            ]
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.systems[0].teams.len(), 2);
        assert_eq!(m.systems[0].teams[0].slug, "developers");
        assert_eq!(m.systems[0].teams[0].permission, "push");
        assert_eq!(m.systems[0].teams[1].slug, "devops");
        assert_eq!(m.systems[0].teams[1].permission, "admin");
    }

    #[test]
    fn manifest_with_rulesets_and_teams() {
        let toml_str = r#"
            [org]
            name = "org"

            [rulesets.branch_protection]
            enabled = true
            enforcement = "active"
            required_approvals = 1
            block_force_pushes = true

            [[systems]]
            id = "be"
            name = "Backend"
            teams = [
                { slug = "dev", permission = "push" },
            ]
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        let bp = m.rulesets.branch_protection.as_ref().unwrap();
        assert!(bp.enabled);
        assert_eq!(bp.enforcement, "active");
        assert_eq!(bp.required_approvals, 1);
        assert!(bp.block_force_pushes);
        assert_eq!(m.systems[0].teams.len(), 1);
    }

    #[test]
    fn security_checks_empty_by_default() {
        let sc: SecurityConfig = toml::from_str("").unwrap();
        assert!(sc.checks.is_empty());
    }

    #[test]
    fn security_checks_file_exists() {
        let toml_str = r#"
            [[checks]]
            name = "Dependabot Config"
            type = "file_exists"
            path = ".github/dependabot.yml"
        "#;
        let sc: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.checks.len(), 1);
        assert_eq!(sc.checks[0].name(), "Dependabot Config");
        assert_eq!(
            sc.checks[0],
            SecurityCheck::FileExists {
                name: "Dependabot Config".into(),
                path: ".github/dependabot.yml".into(),
            }
        );
    }

    #[test]
    fn security_checks_workflow_exists() {
        let toml_str = r#"
            [[checks]]
            name = "CI Pipeline"
            type = "workflow_exists"
            path = ".github/workflows/ci.yml"
        "#;
        let sc: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.checks.len(), 1);
        assert_eq!(sc.checks[0].name(), "CI Pipeline");
        assert!(matches!(
            &sc.checks[0],
            SecurityCheck::WorkflowExists { path, .. } if path == ".github/workflows/ci.yml"
        ));
    }

    #[test]
    fn security_checks_topic_contains() {
        let toml_str = r#"
            [[checks]]
            name = "Managed"
            type = "topic_contains"
            topic = "ward-managed"
        "#;
        let sc: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.checks.len(), 1);
        assert_eq!(sc.checks[0].name(), "Managed");
        assert!(matches!(
            &sc.checks[0],
            SecurityCheck::TopicContains { topic, .. } if topic == "ward-managed"
        ));
    }

    #[test]
    fn security_checks_branch_protection() {
        let toml_str = r#"
            [[checks]]
            name = "Branch Protected"
            type = "branch_protection"
        "#;
        let sc: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.checks.len(), 1);
        assert_eq!(sc.checks[0].name(), "Branch Protected");
        assert!(matches!(
            &sc.checks[0],
            SecurityCheck::BranchProtection { .. }
        ));
    }

    #[test]
    fn security_checks_default_branch() {
        let toml_str = r#"
            [[checks]]
            name = "Main Branch"
            type = "default_branch"
            expected = "main"
        "#;
        let sc: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.checks.len(), 1);
        assert_eq!(sc.checks[0].name(), "Main Branch");
        assert!(matches!(
            &sc.checks[0],
            SecurityCheck::DefaultBranch { expected, .. } if expected == "main"
        ));
    }

    #[test]
    fn security_checks_multiple() {
        let toml_str = r#"
            [[checks]]
            name = "Has CI"
            type = "workflow_exists"
            path = ".github/workflows/ci.yml"

            [[checks]]
            name = "Main Branch"
            type = "default_branch"
            expected = "main"

            [[checks]]
            name = "Tagged"
            type = "topic_contains"
            topic = "managed"
        "#;
        let sc: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.checks.len(), 3);
        assert_eq!(sc.checks[0].name(), "Has CI");
        assert_eq!(sc.checks[1].name(), "Main Branch");
        assert_eq!(sc.checks[2].name(), "Tagged");
    }

    #[test]
    fn manifest_with_security_checks() {
        let toml_str = r#"
            [org]
            name = "org"

            [[security.checks]]
            name = "Dependabot Config"
            type = "file_exists"
            path = ".github/dependabot.yml"

            [[security.checks]]
            name = "Main Branch"
            type = "default_branch"
            expected = "main"
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.security.checks.len(), 2);
        assert_eq!(m.security.checks[0].name(), "Dependabot Config");
        assert_eq!(m.security.checks[1].name(), "Main Branch");
    }

    #[test]
    fn ruleset_bypass_teams_parsing() {
        let toml_str = r#"
            enabled = true
            bypass_teams = ["team-owners", "release-managers"]
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        assert_eq!(rbp.bypass_teams.len(), 2);
        assert_eq!(rbp.bypass_teams[0].slug(), "team-owners");
        assert_eq!(rbp.bypass_teams[0].bypass_mode(), "always");
        assert_eq!(rbp.bypass_teams[1].slug(), "release-managers");
        assert_eq!(rbp.bypass_teams[1].bypass_mode(), "always");
    }

    #[test]
    fn ruleset_bypass_teams_detailed_parsing() {
        let toml_str = r#"
            enabled = true
            bypass_teams = [{ slug = "team-owners", bypass_mode = "pull_request" }]
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        assert_eq!(rbp.bypass_teams.len(), 1);
        assert_eq!(rbp.bypass_teams[0].slug(), "team-owners");
        assert_eq!(rbp.bypass_teams[0].bypass_mode(), "pull_request");
    }

    #[test]
    fn ruleset_bypass_teams_detailed_default_mode() {
        let toml_str = r#"
            enabled = true
            bypass_teams = [{ slug = "team-owners" }]
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        assert_eq!(rbp.bypass_teams.len(), 1);
        assert_eq!(rbp.bypass_teams[0].slug(), "team-owners");
        assert_eq!(rbp.bypass_teams[0].bypass_mode(), "always");
    }

    #[test]
    fn ruleset_bypass_teams_mixed_simple_and_detailed() {
        let toml_str = r#"
            enabled = true
            bypass_teams = ["simple-team", { slug = "detailed-team", bypass_mode = "pull_request" }]
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        assert_eq!(rbp.bypass_teams.len(), 2);
        assert_eq!(rbp.bypass_teams[0].slug(), "simple-team");
        assert_eq!(rbp.bypass_teams[0].bypass_mode(), "always");
        assert_eq!(rbp.bypass_teams[1].slug(), "detailed-team");
        assert_eq!(rbp.bypass_teams[1].bypass_mode(), "pull_request");
    }

    #[test]
    fn manifest_with_bypass_teams() {
        let toml_str = r#"
            [org]
            name = "org"

            [rulesets.branch_protection]
            enabled = true
            required_approvals = 1
            bypass_teams = ["global-owners"]
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        let bp = m.rulesets.branch_protection.as_ref().unwrap();
        assert_eq!(bp.bypass_teams.len(), 1);
        assert_eq!(bp.bypass_teams[0].slug(), "global-owners");
    }

    #[test]
    fn per_system_rulesets_override_bypass_teams() {
        let toml_str = r#"
            [org]
            name = "org"

            [rulesets.branch_protection]
            enabled = true
            required_approvals = 2
            dismiss_stale_reviews = true
            bypass_teams = ["global-owners"]

            [[systems]]
            id = "be"
            name = "Backend"

            [systems.rulesets.branch_protection]
            bypass_teams = ["backend-owners"]
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        let merged = m.rulesets_branch_protection_for_system("be").unwrap();
        // bypass_teams overridden by system
        assert_eq!(merged.bypass_teams.len(), 1);
        assert_eq!(merged.bypass_teams[0].slug(), "backend-owners");
        // other fields fall back to global
        assert_eq!(merged.required_approvals, 2);
        assert!(merged.dismiss_stale_reviews);
        assert!(merged.enabled);
    }

    #[test]
    fn per_system_rulesets_override_multiple_fields() {
        let toml_str = r#"
            [org]
            name = "org"

            [rulesets.branch_protection]
            enabled = true
            required_approvals = 1
            block_force_pushes = true

            [[systems]]
            id = "fe"
            name = "Frontend"

            [systems.rulesets.branch_protection]
            required_approvals = 3
            bypass_teams = ["fe-owners"]
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        let merged = m.rulesets_branch_protection_for_system("fe").unwrap();
        assert_eq!(merged.required_approvals, 3);
        assert_eq!(merged.bypass_teams.len(), 1);
        assert_eq!(merged.bypass_teams[0].slug(), "fe-owners");
        // falls back to global
        assert!(merged.block_force_pushes);
    }

    #[test]
    fn per_system_rulesets_falls_back_to_global_when_no_override() {
        let toml_str = r#"
            [org]
            name = "org"

            [rulesets.branch_protection]
            enabled = true
            required_approvals = 2
            bypass_teams = ["global-owners"]

            [[systems]]
            id = "be"
            name = "Backend"
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        let config = m.rulesets_branch_protection_for_system("be").unwrap();
        assert_eq!(config.required_approvals, 2);
        assert_eq!(config.bypass_teams.len(), 1);
        assert_eq!(config.bypass_teams[0].slug(), "global-owners");
    }

    #[test]
    fn per_system_rulesets_none_when_no_global() {
        let toml_str = r#"
            [org]
            name = "org"

            [[systems]]
            id = "be"
            name = "Backend"

            [systems.rulesets.branch_protection]
            bypass_teams = ["be-owners"]
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        // No global rulesets.branch_protection, so returns None
        assert!(m.rulesets_branch_protection_for_system("be").is_none());
    }

    #[test]
    fn merge_with_all_none_returns_base() {
        let base = RulesetBranchProtection {
            enabled: true,
            name: Some("Base".to_string()),
            enforcement: "active".to_string(),
            required_approvals: 2,
            dismiss_stale_reviews: true,
            require_code_owner_reviews: true,
            required_status_checks: vec!["ci".to_string()],
            require_linear_history: true,
            block_force_pushes: true,
            block_deletions: true,
            bypass_teams: vec![BypassTeam::Simple("global".to_string())],
            overrides: vec![],
        };
        let over = RulesetBranchProtectionOverride::default();
        let merged = base.merge_with(&over);
        assert_eq!(merged, base);
    }

    #[test]
    fn repo_override_pattern_matching() {
        let toml_str = r#"
            enabled = true
            required_approvals = 2
            block_force_pushes = true
            bypass_teams = ["default-admins"]

            [[overrides]]
            repo_patterns = ["*-operations", "*-system"]
            block_force_pushes = false
            bypass_teams = [{ slug = "ops-admins", bypass_mode = "always" }]
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        assert_eq!(rbp.overrides.len(), 1);
        assert_eq!(
            rbp.overrides[0].repo_patterns,
            vec!["*-operations", "*-system"]
        );
    }

    #[test]
    fn for_repo_returns_override_for_matching_repo() {
        let toml_str = r#"
            enabled = true
            required_approvals = 2
            block_force_pushes = true
            bypass_teams = ["default-admins"]

            [[overrides]]
            repo_patterns = ["*-operations", "*-system"]
            block_force_pushes = false
            bypass_teams = [{ slug = "ops-admins", bypass_mode = "always" }]
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        let resolved = rbp.for_repo("my-service-operations");
        assert!(!resolved.block_force_pushes);
        assert_eq!(resolved.bypass_teams.len(), 1);
        assert_eq!(resolved.bypass_teams[0].slug(), "ops-admins");
        assert_eq!(resolved.required_approvals, 2); // falls back to base
        assert!(resolved.overrides.is_empty()); // overrides not carried over
    }

    #[test]
    fn for_repo_returns_base_for_non_matching_repo() {
        let toml_str = r#"
            enabled = true
            required_approvals = 2
            block_force_pushes = true
            bypass_teams = ["default-admins"]

            [[overrides]]
            repo_patterns = ["*-operations", "*-system"]
            block_force_pushes = false
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        let resolved = rbp.for_repo("my-service-api");
        assert!(resolved.block_force_pushes);
        assert_eq!(resolved.bypass_teams.len(), 1);
        assert_eq!(resolved.bypass_teams[0].slug(), "default-admins");
        assert!(resolved.overrides.is_empty());
    }

    #[test]
    fn for_repo_first_override_wins() {
        let toml_str = r#"
            enabled = true
            required_approvals = 1

            [[overrides]]
            repo_patterns = ["*-operations"]
            required_approvals = 3

            [[overrides]]
            repo_patterns = ["*-operations", "*-system"]
            required_approvals = 5
        "#;
        let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
        let resolved = rbp.for_repo("my-service-operations");
        // First override matches, so required_approvals = 3
        assert_eq!(resolved.required_approvals, 3);
    }

    #[test]
    fn full_manifest_with_repo_overrides() {
        let toml_str = r#"
            [org]
            name = "org"

            [rulesets.branch_protection]
            enabled = true
            required_approvals = 1
            dismiss_stale_reviews = true
            block_force_pushes = true
            bypass_teams = [{ slug = "default-admins", bypass_mode = "always" }]

            [[rulesets.branch_protection.overrides]]
            repo_patterns = ["*-operations", "*-system"]
            block_force_pushes = false
            bypass_teams = [{ slug = "ops-admins", bypass_mode = "always" }]

            [[systems]]
            id = "s07439"
            name = "Party Registry"

            [systems.rulesets.branch_protection]
            bypass_teams = [{ slug = "party-owners", bypass_mode = "pull_request" }]

            [[systems.rulesets.branch_protection.overrides]]
            repo_patterns = ["*-operations"]
            bypass_teams = [{ slug = "party-owners", bypass_mode = "always" }]
        "#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        let config = m.rulesets_branch_protection_for_system("s07439").unwrap();

        // system override replaces bypass_teams
        assert_eq!(config.bypass_teams.len(), 1);
        assert_eq!(config.bypass_teams[0].slug(), "party-owners");
        assert_eq!(config.bypass_teams[0].bypass_mode(), "pull_request");

        // system override replaces overrides too
        assert_eq!(config.overrides.len(), 1);
        assert_eq!(config.overrides[0].repo_patterns, vec!["*-operations"]);

        // for_repo on operations repo uses system-level override
        let ops_config = config.for_repo("s07439-operations");
        assert_eq!(ops_config.bypass_teams.len(), 1);
        assert_eq!(ops_config.bypass_teams[0].slug(), "party-owners");
        assert_eq!(ops_config.bypass_teams[0].bypass_mode(), "always");

        // for_repo on non-operations repo uses base system config
        let app_config = config.for_repo("s07439-api");
        assert_eq!(app_config.bypass_teams.len(), 1);
        assert_eq!(app_config.bypass_teams[0].slug(), "party-owners");
        assert_eq!(app_config.bypass_teams[0].bypass_mode(), "pull_request");
    }
}
