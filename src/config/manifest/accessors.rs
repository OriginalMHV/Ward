use std::path::Path;

use anyhow::{Context, Result};

use super::*;

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

    /// Find the system that owns a repo by matching the repo name against system id prefixes.
    /// Returns the system id, or `None` if the repo doesn't match any system.
    pub fn system_for_repo(&self, repo_name: &str) -> Option<&str> {
        self.systems
            .iter()
            .filter(|s| repo_name == s.id || repo_name.starts_with(&format!("{}-", s.id)))
            .max_by_key(|s| s.id.len()) // longest match wins
            .map(|s| s.id.as_str())
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
