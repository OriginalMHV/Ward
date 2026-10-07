use std::path::Path;

use anyhow::{Context, Result};

use super::*;

/// The manifest path Ward reads when `--config` is not given.
pub const DEFAULT_MANIFEST_PATH: &str = "ward.toml";

/// The message for a manifest that does not exist at `path`.
pub fn missing_manifest_message(path: &str) -> String {
    let shown = std::path::absolute(path).map_or_else(
        |_| path.to_owned(),
        |absolute| absolute.display().to_string(),
    );
    format!(
        "No manifest found at {shown}. Run 'ward init' to create one, or 'ward import <owner/repo>' to build one from a repository."
    )
}

impl Manifest {
    pub fn load(path: Option<&str>) -> Result<Self> {
        let explicit = path.is_some();
        let path = path.unwrap_or(DEFAULT_MANIFEST_PATH);

        if !Path::new(path).exists() {
            if explicit {
                anyhow::bail!("{}", missing_manifest_message(path));
            }
            tracing::info!("No ward.toml found, using defaults");
            return Ok(Self::default());
        }

        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {path}"))?;

        let table: toml::Table =
            toml::from_str(&content).with_context(|| format!("Could not read the manifest {path}. It is not valid TOML, or a field has the wrong type."))?;
        if table.contains_key("schema") {
            anyhow::bail!(
                "{path} contains a [schema] table. Ward no longer uses it. Remove the table."
            );
        }

        toml::from_str(&content).with_context(|| format!("Could not read the manifest {path}. It is not valid TOML, or a field has the wrong type."))
    }

    pub fn system(&self, id: &str) -> Option<&SystemConfig> {
        self.systems.iter().find(|s| s.id == id)
    }

    /// Find the system that owns a repo by matching the repo name against system id prefixes.
    /// Returns the system id, or `None` if the repo doesn't match any system.
    pub fn system_for_repo(&self, repo_name: &str) -> Option<&str> {
        self.systems
            .iter()
            .filter(|system| {
                system.repos.iter().any(|repo| repo == repo_name)
                    || (system.match_prefix
                        && (repo_name == system.id
                            || repo_name.starts_with(&format!("{}-", system.id))))
            })
            .max_by_key(|s| s.id.len()) // longest match wins
            .map(|s| s.id.as_str())
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

    pub fn matches_prefix_for_system(&self, system_id: &str) -> bool {
        self.systems
            .iter()
            .find(|s| s.id == system_id)
            .is_none_or(|s| s.match_prefix)
    }

    pub fn categories_for_repo(&self, repo_name: &str) -> ManifestCategories {
        let mut categories = self.categories.clone();
        let Some(system) = self
            .system_for_repo(repo_name)
            .and_then(|system_id| self.system(system_id))
        else {
            return categories;
        };

        let overrides = &system.categories;
        if overrides.security.is_some() {
            categories.security = overrides.security.clone();
        }
        if overrides.repository.is_some() {
            categories.repository = overrides.repository.clone();
        }
        if overrides.branch_protection.is_some() {
            categories.branch_protection = overrides.branch_protection.clone();
        }
        if overrides.rulesets.is_some() {
            categories.rulesets = overrides.rulesets.clone();
        }
        if overrides.files.is_some() {
            categories.files = overrides.files.clone();
        }
        if overrides.actions.is_some() {
            categories.actions = overrides.actions.clone();
        }
        if overrides.environments.is_some() {
            categories.environments = overrides.environments.clone();
        }
        if overrides.access.is_some() {
            categories.access = overrides.access.clone();
        }
        if overrides.integrations.is_some() {
            categories.integrations = overrides.integrations.clone();
        }
        categories
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            org: OrgConfig {
                name: String::new(),
            },
            file_delivery: FileDeliveryConfig::default(),
            systems: Vec::new(),
            provenance: None,
            categories: ManifestCategories::default(),
            coverage: Vec::new(),
        }
    }
}
