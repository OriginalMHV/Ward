use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

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
    pub systems: Vec<SystemConfig>,
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
            systems: Vec::new(),
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
}
