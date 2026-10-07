//! Unified plan/apply orchestration across all manifest categories.
//!
//! This module ties the individual category reconcilers (general/repository,
//! files, security, rulesets, branch protection, actions, environments,
//! access, integrations) into a single plan and a single safe apply order.
//! It never creates, renames, transfers, or deletes repositories: it only
//! reconciles configuration of repositories that already exist and are owned
//! by the configured organization.

mod apply;
mod gating;
mod model;
mod plan;
mod report;
mod targets;
#[cfg(test)]
mod tests;

pub use apply::{apply, apply_prepared};
pub use plan::{PreparedApply, plan, prepare_apply};
pub use report::{CategoryReport, CoverageCounts, CoverageOutcomeCount, RepoReport, UnifiedReport};
pub use targets::{reject_archived_explicit_target, resolve_target_repos};

/// A managed category, addressable by a stable CLI name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum Category {
    #[value(alias = "repo", alias = "general")]
    Repository,
    #[value(alias = "file")]
    Files,
    Security,
    #[value(alias = "ruleset")]
    Rulesets,
    #[value(
        name = "branch-protection",
        alias = "branch_protection",
        alias = "protection"
    )]
    BranchProtection,
    Actions,
    #[value(alias = "environment")]
    Environments,
    #[value(alias = "teams")]
    Access,
    #[value(alias = "integration")]
    Integrations,
}

impl Category {
    /// Stable, user-facing name accepted by `--category`.
    pub fn stable_name(self) -> &'static str {
        match self {
            Category::Repository => "repository",
            Category::Files => "files",
            Category::Security => "security",
            Category::Rulesets => "rulesets",
            Category::BranchProtection => "branch-protection",
            Category::Actions => "actions",
            Category::Environments => "environments",
            Category::Access => "access",
            Category::Integrations => "integrations",
        }
    }

    /// Every category, in the order safe apply must execute.
    ///
    /// Repository/general first, then files (branch + PR), then the settings
    /// categories, and finally rulesets and classic branch protection which
    /// may depend on files landing first.
    pub fn apply_order() -> [Category; 9] {
        [
            Category::Repository,
            Category::Files,
            Category::Security,
            Category::Actions,
            Category::Environments,
            Category::Access,
            Category::Integrations,
            Category::Rulesets,
            Category::BranchProtection,
        ]
    }
}

/// Resolve the `--category` values. An empty input selects all categories.
/// Duplicates are dropped and the order of first appearance is kept.
pub fn select_categories(values: &[Category]) -> Vec<Category> {
    if values.is_empty() {
        return Category::apply_order().to_vec();
    }

    let mut selected = Vec::new();
    for category in values {
        if !selected.contains(category) {
            selected.push(*category);
        }
    }
    selected
}

/// Shared options for both plan and apply.
#[derive(Debug, Clone)]
pub struct UnifiedOptions {
    pub categories: Vec<Category>,
    pub allow_high_impact: bool,
    pub verify: bool,
}

impl UnifiedOptions {
    fn includes(&self, category: Category) -> bool {
        self.categories.contains(&category)
    }
}
