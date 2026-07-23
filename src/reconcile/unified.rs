//! Unified plan/apply orchestration across all manifest v2 categories.
//!
//! This module ties the individual category reconcilers (general/repository,
//! files, security, rulesets, branch protection, actions, environments,
//! access, integrations) into a single plan and a single safe apply order.
//! It never creates, renames, transfers, or deletes repositories: it only
//! reconciles configuration of repositories that already exist and are owned
//! by the configured organization.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::config::Manifest;
use crate::config::manifest::{
    ActionsCategoryV2, BranchProtectionCategoryV2, CategoryPolicy, CoverageEntry, CoverageOutcome,
    EnvironmentsCategoryV2, FilesCategoryV2, ManagementDisposition, RepositoryAccessCategoryV2,
    RepositoryCategoryV2, RepositoryIntegrationsCategoryV2, RulesetsCategoryV2,
};
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::github::repos::Repository;
use crate::reconcile::{access_integrations, actions_environments, files, general, security_rules};

/// A managed category, addressable by a stable CLI name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Repository,
    Files,
    Security,
    Rulesets,
    BranchProtection,
    Actions,
    Environments,
    Access,
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

    /// Parse a stable category name (with a couple of forgiving aliases).
    pub fn parse(value: &str) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        let category = match normalized.as_str() {
            "repository" | "repo" | "general" => Category::Repository,
            "files" | "file" => Category::Files,
            "security" => Category::Security,
            "rulesets" | "ruleset" => Category::Rulesets,
            "branch-protection" | "protection" => Category::BranchProtection,
            "actions" => Category::Actions,
            "environments" | "environment" => Category::Environments,
            "access" | "teams" => Category::Access,
            "integrations" | "integration" => Category::Integrations,
            other => bail!(
                "Unknown category `{other}`. Valid categories: {}",
                Self::all_stable_names().join(", ")
            ),
        };
        Ok(category)
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

    fn all_stable_names() -> Vec<&'static str> {
        Self::apply_order()
            .iter()
            .map(|category| category.stable_name())
            .collect()
    }
}

/// Parse and validate a set of `--category` values. An empty input selects all
/// categories.
pub fn parse_categories(values: &[String]) -> Result<Vec<Category>> {
    if values.is_empty() {
        return Ok(Category::apply_order().to_vec());
    }

    let mut selected = Vec::new();
    for value in values {
        let category = Category::parse(value)?;
        if !selected.contains(&category) {
            selected.push(category);
        }
    }
    Ok(selected)
}

/// Shared options for both plan and apply.
#[derive(Debug, Clone)]
pub struct UnifiedOptions {
    pub categories: Vec<Category>,
    pub allow_high_impact: bool,
}

impl UnifiedOptions {
    fn includes(&self, category: Category) -> bool {
        self.categories.contains(&category)
    }
}

// ---------------------------------------------------------------------------
// Serializable report shape (stable JSON contract)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct CoverageCounts {
    pub total: usize,
    pub collected: usize,
    pub degraded: usize,
    pub not_applicable: usize,
}

#[derive(Debug, Serialize)]
pub struct CoverageOutcomeCount {
    pub outcome: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct CategoryReport {
    pub category: String,
    pub disposition: String,
    pub status: String,
    pub actionable: usize,
    pub blocked: usize,
    pub warnings: usize,
    pub deferred: usize,
    pub coverage: CoverageCounts,
    pub coverage_outcomes: Vec<CoverageOutcomeCount>,
    pub details: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(skip_serializing_if = "is_false")]
    pub configuration_pull_request_pending: bool,
}

#[derive(Debug, Serialize)]
pub struct RepoReport {
    pub repo: String,
    pub categories: Vec<CategoryReport>,
    pub actionable: usize,
    pub blocked: usize,
    pub warnings: usize,
    pub deferred: usize,
}

#[derive(Debug, Serialize)]
pub struct UnifiedReport {
    pub repos: Vec<RepoReport>,
    pub actionable: usize,
    pub blocked: usize,
    pub warnings: usize,
    pub deferred: usize,
    pub coverage: CoverageCounts,
}

impl UnifiedReport {
    pub fn from_repos(repos: Vec<RepoReport>) -> Self {
        let mut coverage = CoverageCounts::default();
        let mut actionable = 0;
        let mut blocked = 0;
        let mut warnings = 0;
        let mut deferred = 0;
        for repo in &repos {
            actionable += repo.actionable;
            blocked += repo.blocked;
            warnings += repo.warnings;
            deferred += repo.deferred;
            for category in &repo.categories {
                coverage.total += category.coverage.total;
                coverage.collected += category.coverage.collected;
                coverage.degraded += category.coverage.degraded;
                coverage.not_applicable += category.coverage.not_applicable;
            }
        }
        Self {
            repos,
            actionable,
            blocked,
            warnings,
            deferred,
            coverage,
        }
    }

    /// Whether any category is blocked or failed (drives a non-zero exit).
    pub fn has_failures(&self) -> bool {
        self.blocked > 0
            || self.repos.iter().any(|repo| {
                repo.categories
                    .iter()
                    .any(|category| category.status == "failed")
            })
    }
}

// ---------------------------------------------------------------------------
// Internal typed plan (retained so apply can execute without re-planning)
// ---------------------------------------------------------------------------

enum CategoryPlanKind {
    Repository(Box<general::GeneralPlan>),
    Files(files::FilesPlan),
    Security(security_rules::SecurityPlan),
    Rulesets(security_rules::RulesetsPlan),
    BranchProtection(security_rules::BranchProtectionPlan),
    Actions(actions_environments::ActionsPlan),
    Environments(actions_environments::EnvironmentsPlan),
    Access(access_integrations::AccessPlan),
    Integrations(access_integrations::IntegrationsPlan),
    /// Collection failed. Surfaced as an explicit blocked category, never hidden.
    CollectionFailed(String),
    /// Category is not present in the manifest at all.
    Absent,
}

struct CategoryPlan {
    name: Category,
    disposition: ManagementDisposition,
    is_blocked_collection: bool,
    coverage: Vec<CoverageEntry>,
    actionable: usize,
    blocked: usize,
    warnings: usize,
    details: Vec<String>,
    kind: CategoryPlanKind,
}

impl CategoryPlan {
    fn disposition_label(&self) -> String {
        if self.is_blocked_collection {
            return "blocked".to_owned();
        }
        disposition_label(self.disposition)
    }

    fn plan_status(&self) -> &'static str {
        if self.is_blocked_collection {
            return "blocked";
        }
        match self.kind {
            CategoryPlanKind::Absent => "skipped",
            _ => {
                if self.blocked > 0 {
                    "blocked"
                } else if self.disposition != ManagementDisposition::Managed {
                    "observed"
                } else if self.actionable > 0 {
                    "planned"
                } else {
                    "noop"
                }
            }
        }
    }

    fn to_report(&self, status: &str, deferred: usize, verified: Option<bool>) -> CategoryReport {
        let error = match &self.kind {
            CategoryPlanKind::CollectionFailed(message) => Some(message.clone()),
            _ => None,
        };
        CategoryReport {
            category: self.name.stable_name().to_owned(),
            disposition: self.disposition_label(),
            status: status.to_owned(),
            actionable: self.actionable,
            blocked: self.blocked,
            warnings: self.warnings,
            deferred,
            coverage: coverage_counts(&self.coverage),
            coverage_outcomes: coverage_outcomes(&self.coverage),
            details: self.details.clone(),
            error,
            verified,
            configuration_pull_request_pending: false,
        }
    }
}

struct RepoPlan {
    repo: String,
    default_branch: String,
    categories: Vec<CategoryPlan>,
}

impl RepoPlan {
    #[cfg(test)]
    fn to_report(&self) -> RepoReport {
        self.to_report_with_config_pr(false)
    }

    fn to_report_with_config_pr(&self, existing_config_pr: bool) -> RepoReport {
        let config_pr_pending = existing_config_pr || self.plans_config_pull_request();
        let categories: Vec<CategoryReport> = self
            .categories
            .iter()
            .map(|category| {
                let report = category.to_report(category.plan_status(), 0, None);
                adjust_for_config_pr(report, category, config_pr_pending, true)
            })
            .collect();
        aggregate_repo(self.repo.clone(), categories)
    }

    fn plans_config_pull_request(&self) -> bool {
        self.categories.iter().any(|category| {
            category.name == Category::Files
                && category.disposition == ManagementDisposition::Managed
                && category.actionable > 0
                && category.blocked == 0
        })
    }

    fn has_file_dependent_changes(&self) -> bool {
        self.categories
            .iter()
            .any(|category| dependency_deferral(category, true).total > 0)
    }
}

#[derive(Debug, Default)]
struct DependencyDeferral {
    actionable: usize,
    blockers: usize,
    total: usize,
}

fn aggregate_repo(repo: String, categories: Vec<CategoryReport>) -> RepoReport {
    let actionable = categories.iter().map(|c| c.actionable).sum();
    let blocked = categories.iter().map(|c| c.blocked).sum();
    let warnings = categories.iter().map(|c| c.warnings).sum();
    let deferred = categories.iter().map(|c| c.deferred).sum();
    RepoReport {
        repo,
        categories,
        actionable,
        blocked,
        warnings,
        deferred,
    }
}

// ---------------------------------------------------------------------------
// Target repository resolution (same-owner, existing repos only)
// ---------------------------------------------------------------------------

/// Resolve the set of existing, same-owner repositories to reconcile from the
/// global `--repo` / `--system` selectors, falling back to every configured
/// system. Never creates repositories.
pub async fn resolve_target_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<Repository>> {
    if let Some(repo_name) = repo {
        let repository = client.get_repo(repo_name).await?;
        return Ok(vec![repository]);
    }

    let system_ids: Vec<String> = if let Some(system_id) = system {
        vec![system_id.to_owned()]
    } else {
        manifest.systems.iter().map(|s| s.id.clone()).collect()
    };

    if system_ids.is_empty() {
        bail!(
            "No target selected. Pass --repo <name>, --system <id>, or configure systems in ward.toml"
        );
    }

    let mut repos: Vec<Repository> = Vec::new();
    for system_id in &system_ids {
        let excludes = manifest.exclude_patterns_for_system(system_id);
        let explicit = manifest.explicit_repos_for_system(system_id);
        let found = client
            .list_repos_for_system(
                system_id,
                manifest.matches_prefix_for_system(system_id),
                &excludes,
                &explicit,
            )
            .await?;
        for repository in found {
            if !repos
                .iter()
                .any(|existing| existing.name == repository.name)
            {
                repos.push(repository);
            }
        }
    }

    Ok(repos)
}

// ---------------------------------------------------------------------------
// Desired-state assembly
// ---------------------------------------------------------------------------

/// Build the combined general/repository desired state: the manifest v2
/// repository category plus labels stored in the integrations category. The
/// repository category policy governs whether those labels are managed.
fn build_general_desired(manifest: &Manifest) -> Option<general::GeneralDesiredState> {
    let repository = manifest.v2_categories().repository.clone()?;
    let mut desired = general::GeneralDesiredState::from(repository);
    if let Some(integrations) = manifest.v2_categories().integrations.as_ref() {
        desired.labels = integrations
            .labels
            .iter()
            .cloned()
            .map(general::GeneralLabel::from)
            .collect();
    }
    Some(desired)
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

/// Plan every selected category for every target repository.
pub async fn plan(
    client: &Client,
    manifest: &Manifest,
    repos: &[Repository],
    options: &UnifiedOptions,
) -> Result<UnifiedReport> {
    let mut repo_reports = Vec::with_capacity(repos.len());
    let branch = sync_branch(manifest);
    for repository in repos {
        let plan = plan_repo(client, manifest, repository, options).await;
        let existing_config_pr =
            if !plan.plans_config_pull_request() && plan.has_file_dependent_changes() {
                client
                    .find_open_pull_request(&plan.repo, &branch)
                    .await?
                    .is_some()
            } else {
                false
            };
        repo_reports.push(plan.to_report_with_config_pr(existing_config_pr));
    }
    Ok(UnifiedReport::from_repos(repo_reports))
}

async fn plan_repo(
    client: &Client,
    manifest: &Manifest,
    repository: &Repository,
    options: &UnifiedOptions,
) -> RepoPlan {
    let repo = repository.name.clone();
    let mut categories = Vec::new();

    for category in Category::apply_order() {
        if !options.includes(category) {
            continue;
        }
        let planned = plan_category(client, manifest, &repo, category, options).await;
        categories.push(planned);
    }

    RepoPlan {
        repo,
        default_branch: repository.default_branch.clone(),
        categories,
    }
}

async fn plan_category(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: Category,
    options: &UnifiedOptions,
) -> CategoryPlan {
    match category {
        Category::Repository => plan_repository(client, manifest, repo, options).await,
        Category::Files => plan_files(client, manifest, repo).await,
        Category::Security => plan_security(client, manifest, repo).await,
        Category::Rulesets => plan_rulesets(client, manifest, repo).await,
        Category::BranchProtection => plan_branch_protection(client, manifest, repo).await,
        Category::Actions => plan_actions(client, manifest, repo).await,
        Category::Environments => plan_environments(client, manifest, repo).await,
        Category::Access => plan_access(client, manifest, repo).await,
        Category::Integrations => plan_integrations(client, manifest, repo).await,
    }
}

fn absent_category(name: Category) -> CategoryPlan {
    CategoryPlan {
        name,
        disposition: ManagementDisposition::Observe,
        is_blocked_collection: false,
        coverage: Vec::new(),
        actionable: 0,
        blocked: 0,
        warnings: 0,
        details: Vec::new(),
        kind: CategoryPlanKind::Absent,
    }
}

fn collection_failed(
    name: Category,
    disposition: ManagementDisposition,
    error: anyhow::Error,
) -> CategoryPlan {
    let message = format!("{error:#}");
    CategoryPlan {
        name,
        disposition,
        is_blocked_collection: true,
        coverage: Vec::new(),
        actionable: 0,
        blocked: 1,
        warnings: 0,
        details: vec![format!("collection failed: {message}")],
        kind: CategoryPlanKind::CollectionFailed(message),
    }
}

async fn plan_repository(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    options: &UnifiedOptions,
) -> CategoryPlan {
    let Some(desired) = build_general_desired(manifest) else {
        return absent_category(Category::Repository);
    };
    let disposition = desired.repository.policy.disposition;
    let current = match general::collect(client, repo).await {
        Ok(state) => state,
        Err(error) => return collection_failed(Category::Repository, disposition, error),
    };
    let plan_options = general::GeneralPlanOptions {
        allow_high_impact: options.allow_high_impact,
    };
    let plan = general::plan_with_options(repo, &desired, &current, plan_options);

    let actionable = plan.changes.len();
    let blocked = plan.blocked_changes.len();
    let warnings = degraded_coverage(&plan.coverage);
    let mut details: Vec<String> = plan
        .changes
        .iter()
        .take(DETAIL_LIMIT)
        .map(describe_general_change)
        .collect();
    for change in &plan.blocked_changes {
        details.push(format!(
            "blocked (high-impact): {}",
            describe_general_change(change)
        ));
    }
    let coverage = plan.coverage.clone();

    CategoryPlan {
        name: Category::Repository,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Repository(Box::new(plan)),
    }
}

async fn plan_files(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().files.clone() else {
        return absent_category(Category::Files);
    };
    let disposition = desired.policy.disposition;
    let collection = match files::collect_files_category(client, repo, None, Some(&desired)).await {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::Files, disposition, error),
    };
    let coverage = collection.coverage.clone();
    let plan = match files::plan_files_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::Files, disposition, error),
    };

    let actionable = plan.atomic_entries.len();
    let blocked = plan
        .issues
        .iter()
        .filter(|issue| issue.severity == files::FilesIssueSeverity::Blocker)
        .count();
    let warnings = plan
        .issues
        .iter()
        .filter(|issue| issue.severity == files::FilesIssueSeverity::Warning)
        .count()
        + degraded_coverage(&coverage);
    let mut details = Vec::new();
    if actionable > 0 {
        details.push(format!("{actionable} file change(s) via pull request"));
    }
    for issue in plan.issues.iter().take(DETAIL_LIMIT) {
        details.push(format!("{:?}: {}", issue.severity, issue.message));
    }

    CategoryPlan {
        name: Category::Files,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Files(plan),
    }
}

async fn plan_security(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().security.clone() else {
        return absent_category(Category::Security);
    };
    let disposition = desired.policy.disposition;
    let collection =
        match security_rules::collect_security_category(client, repo, Some(&desired)).await {
            Ok(collection) => collection,
            Err(error) => return collection_failed(Category::Security, disposition, error),
        };
    let coverage = collection.coverage.clone();
    let plan = match security_rules::plan_security_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::Security, disposition, error),
    };

    let actionable = usize::from(plan.has_changes());
    let (blocked, warnings) = reconcile_issue_counts(&plan.issues);
    let warnings = warnings + degraded_coverage(&coverage);
    let details = security_change_details(&plan);

    CategoryPlan {
        name: Category::Security,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Security(plan),
    }
}

async fn plan_rulesets(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().rulesets.clone() else {
        return absent_category(Category::Rulesets);
    };
    let disposition = desired.policy.disposition;
    let collection =
        match security_rules::collect_rulesets_category(client, repo, Some(&desired)).await {
            Ok(collection) => collection,
            Err(error) => return collection_failed(Category::Rulesets, disposition, error),
        };
    let coverage = collection.coverage.clone();
    let plan = match security_rules::plan_rulesets_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::Rulesets, disposition, error),
    };

    let actionable = plan
        .actions
        .iter()
        .filter(|action| !matches!(action, security_rules::RulesetPlanAction::Unchanged { .. }))
        .count();
    let (blocked, warnings) = reconcile_issue_counts(&plan.issues);
    let warnings = warnings + degraded_coverage(&coverage);
    let details = ruleset_action_details(&plan);

    CategoryPlan {
        name: Category::Rulesets,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Rulesets(plan),
    }
}

async fn plan_branch_protection(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().branch_protection.clone() else {
        return absent_category(Category::BranchProtection);
    };
    let disposition = desired.policy.disposition;
    let collection = match security_rules::collect_branch_protection_category(
        client,
        repo,
        Some(&desired),
    )
    .await
    {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::BranchProtection, disposition, error),
    };
    let coverage = collection.coverage.clone();
    let plan = match security_rules::plan_branch_protection_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::BranchProtection, disposition, error),
    };

    let actionable = plan
        .actions
        .iter()
        .filter(|action| {
            !matches!(
                action,
                security_rules::BranchProtectionPlanAction::Unchanged { .. }
            )
        })
        .count();
    let (blocked, warnings) = reconcile_issue_counts(&plan.issues);
    let warnings = warnings + degraded_coverage(&coverage);
    let details = branch_protection_details(&plan);

    CategoryPlan {
        name: Category::BranchProtection,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::BranchProtection(plan),
    }
}

async fn plan_actions(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().actions.clone() else {
        return absent_category(Category::Actions);
    };
    let disposition = desired.policy.disposition;
    let collection =
        match actions_environments::collect_actions_category(client, repo, Some(&desired)).await {
            Ok(collection) => collection,
            Err(error) => return collection_failed(Category::Actions, disposition, error),
        };
    let coverage = collection.coverage.clone();
    let plan = actions_environments::plan_actions_category(&desired, &collection);

    let actionable = actions_actionable_count(&plan);
    let (blocked, warnings) = actions_issue_counts(&plan.issues);
    let warnings = warnings + degraded_coverage(&coverage);
    let details = actions_details(&plan);

    CategoryPlan {
        name: Category::Actions,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Actions(plan),
    }
}

async fn plan_environments(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().environments.clone() else {
        return absent_category(Category::Environments);
    };
    let disposition = desired.policy.disposition;
    let collection =
        match actions_environments::collect_environments_category(client, repo, Some(&desired))
            .await
        {
            Ok(collection) => collection,
            Err(error) => return collection_failed(Category::Environments, disposition, error),
        };
    let coverage = collection.coverage.clone();
    let plan = actions_environments::plan_environments_category(&desired, &collection);

    let actionable = environments_actionable_count(&plan);
    let (blocked, warnings) = actions_issue_counts(&plan.issues);
    let warnings = warnings + degraded_coverage(&coverage);
    let mut details = Vec::new();
    for name in plan.environment_deletions.iter().take(DETAIL_LIMIT) {
        details.push(format!("delete environment {name}"));
    }
    for env in plan.environment_plans.iter().take(DETAIL_LIMIT) {
        if env.has_actionable_changes() {
            details.push(format!("update environment {}", env.name));
        }
    }

    CategoryPlan {
        name: Category::Environments,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Environments(plan),
    }
}

async fn plan_access(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().access.clone() else {
        return absent_category(Category::Access);
    };
    let disposition = desired.policy.disposition;
    let collection = match access_integrations::collect_access(client, repo, &desired).await {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::Access, disposition, error),
    };
    let coverage = collection.coverage.clone();
    let plan = access_integrations::plan_access(&collection, &desired);

    let actionable =
        plan.team_actions.len() + plan.collaborator_actions.len() + plan.reference_actions.len();
    let (blocked, warnings) = actions_issue_counts(&plan.issues);
    let warnings = warnings + degraded_coverage(&coverage);
    let mut details: Vec<String> = plan.notes.iter().take(DETAIL_LIMIT).cloned().collect();
    if !plan.team_actions.is_empty() {
        details.push(format!("{} team change(s)", plan.team_actions.len()));
    }
    if !plan.collaborator_actions.is_empty() {
        details.push(format!(
            "{} collaborator change(s)",
            plan.collaborator_actions.len()
        ));
    }

    CategoryPlan {
        name: Category::Access,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Access(plan),
    }
}

async fn plan_integrations(client: &Client, manifest: &Manifest, repo: &str) -> CategoryPlan {
    let Some(desired) = manifest.v2_categories().integrations.clone() else {
        return absent_category(Category::Integrations);
    };
    let disposition = desired.policy.disposition;
    let collection = match access_integrations::collect_integrations(client, repo, &desired).await {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::Integrations, disposition, error),
    };
    let coverage = collection.coverage.clone();
    let plan = access_integrations::plan_integrations(&collection, &desired);

    let actionable = plan.webhook_actions.len()
        + plan.deploy_key_actions.len()
        + plan.autolink_actions.len()
        + usize::from(plan.pages_action.is_some());
    let (blocked, warnings) = actions_issue_counts(&plan.issues);
    let warnings = warnings + degraded_coverage(&coverage);
    let mut details: Vec<String> = plan.notes.iter().take(DETAIL_LIMIT).cloned().collect();
    if !plan.webhook_actions.is_empty() {
        details.push(format!("{} webhook change(s)", plan.webhook_actions.len()));
    }
    if plan.pages_action.is_some() {
        details.push("pages change".to_owned());
    }

    CategoryPlan {
        name: Category::Integrations,
        disposition,
        is_blocked_collection: false,
        coverage,
        actionable,
        blocked,
        warnings,
        details,
        kind: CategoryPlanKind::Integrations(plan),
    }
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// Plan and apply every selected category for every target repository in the
/// safe order, verify results, and emit a structured audit trail.
pub async fn apply(
    client: &Client,
    manifest: &Manifest,
    repos: &[Repository],
    options: &UnifiedOptions,
    audit: &AuditLog,
) -> Result<UnifiedReport> {
    // Complete every read-only plan and dependency preflight before the first
    // mutation so a later repository cannot surprise a partially applied run.
    let mut plans = Vec::with_capacity(repos.len());
    for repository in repos {
        plans.push(plan_repo(client, manifest, repository, options).await);
    }

    let branch = sync_branch(manifest);
    let mut prepared = Vec::with_capacity(plans.len());
    for plan in plans {
        let existing_config_pr = if plan.has_file_dependent_changes() {
            client
                .find_open_pull_request(&plan.repo, &branch)
                .await?
                .is_some()
        } else {
            false
        };
        prepared.push((plan, existing_config_pr));
    }

    let mut repo_reports = Vec::with_capacity(prepared.len());
    for (plan, existing_config_pr) in prepared {
        let report = apply_repo(client, manifest, plan, existing_config_pr, audit).await;
        repo_reports.push(report);
    }
    Ok(UnifiedReport::from_repos(repo_reports))
}

async fn apply_repo(
    client: &Client,
    manifest: &Manifest,
    plan: RepoPlan,
    existing_config_pr: bool,
    audit: &AuditLog,
) -> RepoReport {
    let repo = plan.repo.clone();
    let default_branch = plan.default_branch.clone();
    let branch = sync_branch(manifest);
    let commit_prefix = commit_prefix(manifest);

    let mut config_pr_pending = existing_config_pr;
    let mut reports = Vec::with_capacity(plan.categories.len());

    for category in plan.categories {
        let report = apply_category(
            client,
            manifest,
            &repo,
            &default_branch,
            &branch,
            &commit_prefix,
            &category,
            config_pr_pending,
            audit,
        )
        .await;

        // A config pull request is pending once file changes were routed
        // through it. Downstream categories that depend on those files must be
        // deferred until the PR merges.
        if report.configuration_pull_request_pending {
            config_pr_pending = true;
        }

        reports.push(report);
    }

    aggregate_repo(repo, reports)
}

#[allow(clippy::too_many_arguments)]
async fn apply_category(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    default_branch: &str,
    branch: &str,
    commit_prefix: &str,
    category: &CategoryPlan,
    config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    // Never silently proceed past a failed collection.
    if let CategoryPlanKind::CollectionFailed(message) = &category.kind {
        audit_category(audit, repo, category, "blocked", 0, Some(message.clone()));
        return category.to_report("blocked", 0, None);
    }
    if matches!(category.kind, CategoryPlanKind::Absent) {
        return category.to_report("skipped", 0, None);
    }
    // Observe / reference / placeholder never mutate; only coverage/state.
    if category.disposition != ManagementDisposition::Managed {
        return category.to_report("observed", 0, None);
    }
    let deferral = dependency_deferral(category, config_pr_pending);
    let effective_blocked = category.blocked.saturating_sub(deferral.blockers);
    let effective_actionable = category.actionable.saturating_sub(deferral.actionable);
    // A blocker issue prevents mutation of this category.
    if effective_blocked > 0 {
        audit_category(audit, repo, category, "blocked", 0, None);
        let report = category.to_report("blocked", 0, None);
        return adjust_for_config_pr(report, category, config_pr_pending, false);
    }
    if effective_actionable == 0 {
        let status = if deferral.total > 0 {
            "deferred"
        } else {
            "noop"
        };
        if deferral.total > 0 {
            audit_category(audit, repo, category, "deferred", deferral.total, None);
        }
        let report = category.to_report(status, 0, None);
        return adjust_for_config_pr(report, category, config_pr_pending, false);
    }

    let report = match &category.kind {
        CategoryPlanKind::Repository(plan) => {
            apply_repository(client, repo, category, plan, audit).await
        }
        CategoryPlanKind::Files(plan) => {
            apply_files(
                client,
                manifest,
                repo,
                default_branch,
                branch,
                commit_prefix,
                category,
                plan,
                audit,
            )
            .await
        }
        CategoryPlanKind::Security(plan) => {
            apply_security(client, manifest, repo, category, plan, audit).await
        }
        CategoryPlanKind::Actions(plan) => {
            apply_actions(
                client,
                manifest,
                repo,
                category,
                plan,
                config_pr_pending,
                audit,
            )
            .await
        }
        CategoryPlanKind::Environments(plan) => {
            apply_environments(client, manifest, repo, category, plan, audit).await
        }
        CategoryPlanKind::Access(plan) => {
            apply_access(client, manifest, repo, category, plan, audit).await
        }
        CategoryPlanKind::Integrations(plan) => {
            apply_integrations(
                client,
                manifest,
                repo,
                category,
                plan,
                config_pr_pending,
                audit,
            )
            .await
        }
        CategoryPlanKind::Rulesets(plan) => {
            apply_rulesets(
                client,
                manifest,
                repo,
                category,
                plan,
                config_pr_pending,
                audit,
            )
            .await
        }
        CategoryPlanKind::BranchProtection(plan) => {
            apply_branch_protection(
                client,
                manifest,
                repo,
                category,
                plan,
                config_pr_pending,
                audit,
            )
            .await
        }
        CategoryPlanKind::CollectionFailed(_) | CategoryPlanKind::Absent => {
            category.to_report("skipped", 0, None)
        }
    };
    adjust_for_config_pr(report, category, config_pr_pending, false)
}

async fn apply_repository(
    client: &Client,
    repo: &str,
    category: &CategoryPlan,
    plan: &general::GeneralPlan,
    audit: &AuditLog,
) -> CategoryReport {
    // `general::apply` applies and verifies in one step.
    match general::apply(client, plan).await {
        Ok(verification) => {
            let verified = verification.compliant;
            audit_category(audit, repo, category, "success", 0, None);
            category.to_report("success", 0, Some(verified))
        }
        Err(error) => failure(audit, repo, category, error),
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_files(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    default_branch: &str,
    branch: &str,
    commit_prefix: &str,
    category: &CategoryPlan,
    _default_branch_plan: &files::FilesPlan,
    audit: &AuditLog,
) -> CategoryReport {
    let Some(desired) = manifest.v2_categories().files.clone() else {
        return category.to_report("skipped", 0, None);
    };

    // Never write files to the default branch: route through a dedicated
    // branch and a pull request.
    if let Err(error) = client
        .ensure_dedicated_branch(repo, branch, default_branch)
        .await
    {
        return failure(audit, repo, category, error);
    }

    // Re-collect and re-plan against the dedicated branch so we only commit
    // changes still missing there.
    let branch_collection =
        match files::collect_files_category(client, repo, Some(branch), Some(&desired)).await {
            Ok(collection) => collection,
            Err(error) => return failure(audit, repo, category, error),
        };
    let branch_plan = match files::plan_files_category(&desired, &branch_collection) {
        Ok(plan) => plan,
        Err(error) => return failure(audit, repo, category, error),
    };

    let message = format!("{commit_prefix}sync managed files");

    if !branch_plan.atomic_entries.is_empty() {
        if let Err(error) =
            files::apply_files_plan(client, repo, branch, &message, &branch_plan).await
        {
            return failure(audit, repo, category, error);
        }
    }

    let pr = match client
        .create_pull_request(
            repo,
            &message,
            "Automated managed-file synchronization by Ward.",
            branch,
            default_branch,
            &manifest.file_delivery.reviewers,
        )
        .await
    {
        Ok(pr) => pr,
        Err(error) => return failure(audit, repo, category, error),
    };

    let mut details = category.details.clone();
    details.push(format!("pull request: {}", pr.html_url));

    // Verify files against the PR branch, never the default branch.
    let verified = match files::verify_files_category(client, repo, Some(branch), &desired).await {
        Ok(result) => result.matches,
        Err(error) => {
            let mut report = failure(audit, repo, category, error);
            report.details = details;
            report.configuration_pull_request_pending = true;
            return report;
        }
    };

    if !verified {
        details.push("PR branch does not match desired state after commit".to_owned());
    }

    let status = if verified { "success" } else { "failed" };
    if let Err(error) = audit.log_values(
        repo,
        "apply.files",
        status,
        serde_json::json!({ "actionable": category.actionable }),
        serde_json::json!({ "pull_request": pr.html_url, "branch": branch }),
    ) {
        tracing::warn!(%error, repo, "Failed to write Ward audit entry");
    }

    files_apply_report(category, details, verified)
}

fn files_apply_report(
    category: &CategoryPlan,
    details: Vec<String>,
    verified: bool,
) -> CategoryReport {
    let status = if verified { "success" } else { "failed" };
    CategoryReport {
        details,
        verified: Some(verified),
        configuration_pull_request_pending: true,
        ..category.to_report(status, 0, Some(verified))
    }
}

async fn apply_security(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: &CategoryPlan,
    plan: &security_rules::SecurityPlan,
    audit: &AuditLog,
) -> CategoryReport {
    if let Err(error) = security_rules::apply_security_plan(client, repo, plan).await {
        return failure(audit, repo, category, error);
    }
    let verified = if let Some(desired) = manifest.v2_categories().security.as_ref() {
        match security_rules::verify_security_category(client, repo, desired).await {
            Ok(result) => Some(result.matches),
            Err(error) => return failure(audit, repo, category, error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

#[allow(clippy::too_many_arguments)]
async fn apply_actions(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: &CategoryPlan,
    plan: &actions_environments::ActionsPlan,
    config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    // When a configuration PR is pending, apply only the settings, variables,
    // secrets, and references that do not depend on workflow files landing.
    let (safe_plan, deferred) = actions_plan_for_apply(plan, config_pr_pending);

    match actions_environments::apply_actions_plan(client, repo, &safe_plan).await {
        Ok(result) => {
            if let Some(issue) = result
                .issues
                .iter()
                .find(|issue| issue.severity == actions_environments::IssueSeverity::Blocker)
            {
                return blocked_with_message(audit, repo, category, issue.message.clone());
            }
        }
        Err(error) => return failure(audit, repo, category, error),
    }

    let verified = if let Some(desired) = manifest.v2_categories().actions.as_ref() {
        if config_pr_pending {
            match verify_actions_safe_subset(client, repo, desired).await {
                Ok(matches) => Some(matches),
                Err(error) => return failure(audit, repo, category, error),
            }
        } else {
            match actions_environments::verify_actions_category(client, repo, desired).await {
                Ok(result) => Some(result.compliant),
                Err(error) => return failure(audit, repo, category, error),
            }
        }
    } else {
        None
    };

    finish_success_deferred(audit, repo, category, verified, deferred, "success")
}

async fn apply_environments(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: &CategoryPlan,
    plan: &actions_environments::EnvironmentsPlan,
    audit: &AuditLog,
) -> CategoryReport {
    match actions_environments::apply_environments_plan(client, repo, plan).await {
        Ok(result) => {
            if let Some(issue) = result
                .issues
                .iter()
                .find(|issue| issue.severity == actions_environments::IssueSeverity::Blocker)
            {
                return blocked_with_message(audit, repo, category, issue.message.clone());
            }
        }
        Err(error) => return failure(audit, repo, category, error),
    }
    let verified = if let Some(desired) = manifest.v2_categories().environments.as_ref() {
        match actions_environments::verify_environments_category(client, repo, desired).await {
            Ok(result) => Some(result.compliant),
            Err(error) => return failure(audit, repo, category, error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

async fn apply_access(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: &CategoryPlan,
    plan: &access_integrations::AccessPlan,
    audit: &AuditLog,
) -> CategoryReport {
    let report = match access_integrations::apply_access(client, repo, plan).await {
        Ok(report) => report,
        Err(error) => return failure(audit, repo, category, error),
    };
    if !report.blocked.is_empty() {
        return blocked_with_message(audit, repo, category, report.blocked.join("; "));
    }
    let verified = if let Some(desired) = manifest.v2_categories().access.as_ref() {
        match access_integrations::verify_access(client, repo, desired).await {
            Ok(result) => Some(result.is_ok()),
            Err(error) => return failure(audit, repo, category, error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

#[allow(clippy::too_many_arguments)]
async fn apply_integrations(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: &CategoryPlan,
    plan: &access_integrations::IntegrationsPlan,
    config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    let (safe_plan, deferred) = integrations_plan_for_apply(plan, config_pr_pending);

    let report = match access_integrations::apply_integrations(client, repo, &safe_plan).await {
        Ok(report) => report,
        Err(error) => return failure(audit, repo, category, error),
    };
    if !report.blocked.is_empty() {
        return blocked_with_message(audit, repo, category, report.blocked.join("; "));
    }
    let verified = if let Some(desired) = manifest.v2_categories().integrations.as_ref() {
        if config_pr_pending {
            match verify_integrations_safe_subset(client, repo, desired).await {
                Ok(matches) => Some(matches),
                Err(error) => return failure(audit, repo, category, error),
            }
        } else {
            match access_integrations::verify_integrations(client, repo, desired).await {
                Ok(result) => Some(result.is_ok()),
                Err(error) => return failure(audit, repo, category, error),
            }
        }
    } else {
        None
    };
    finish_success_deferred(audit, repo, category, verified, deferred, "success")
}

#[allow(clippy::too_many_arguments)]
async fn apply_rulesets(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: &CategoryPlan,
    plan: &security_rules::RulesetsPlan,
    _config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    if let Err(error) = security_rules::apply_rulesets_plan(client, repo, plan).await {
        return failure(audit, repo, category, error);
    }
    let verified = if let Some(desired) = manifest.v2_categories().rulesets.as_ref() {
        match security_rules::verify_rulesets_category(client, repo, desired).await {
            Ok(result) => Some(result.matches),
            Err(error) => return failure(audit, repo, category, error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

#[allow(clippy::too_many_arguments)]
async fn apply_branch_protection(
    client: &Client,
    manifest: &Manifest,
    repo: &str,
    category: &CategoryPlan,
    plan: &security_rules::BranchProtectionPlan,
    _config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    if let Err(error) = security_rules::apply_branch_protection_plan(client, repo, plan).await {
        return failure(audit, repo, category, error);
    }
    let verified = if let Some(desired) = manifest.v2_categories().branch_protection.as_ref() {
        match security_rules::verify_branch_protection_category(client, repo, desired).await {
            Ok(result) => Some(result.matches),
            Err(error) => return failure(audit, repo, category, error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

// ---------------------------------------------------------------------------
// Apply-side helpers
// ---------------------------------------------------------------------------

fn finish_success(
    audit: &AuditLog,
    repo: &str,
    category: &CategoryPlan,
    verified: Option<bool>,
) -> CategoryReport {
    if verified == Some(false) {
        audit_category(audit, repo, category, "failed", 0, None);
        let mut report = category.to_report("failed", 0, Some(false));
        report.details.push("verification failed".to_owned());
        return report;
    }
    audit_category(audit, repo, category, "success", 0, None);
    category.to_report("success", 0, verified)
}

fn finish_success_deferred(
    audit: &AuditLog,
    repo: &str,
    category: &CategoryPlan,
    verified: Option<bool>,
    deferred: usize,
    status: &str,
) -> CategoryReport {
    if verified == Some(false) {
        audit_category(audit, repo, category, "failed", deferred, None);
        let mut report = category.to_report("failed", deferred, Some(false));
        report.details.push("verification failed".to_owned());
        return report;
    }
    if deferred > 0 {
        audit_category(audit, repo, category, "deferred", deferred, None);
    } else {
        audit_category(audit, repo, category, "success", 0, None);
    }
    let mut report = category.to_report(status, deferred, verified);
    if deferred > 0 {
        report
            .details
            .push(format!("{deferred} change(s) deferred to configuration PR"));
    }
    report
}

/// Clone an actions plan and clear the workflow enable/disable toggles so the
/// safe subset (settings/variables/secrets/references) can be applied while the
/// toggles are deferred. Returns the safe plan and the deferred count.
fn actions_safe_subset(
    plan: &actions_environments::ActionsPlan,
) -> (actions_environments::ActionsPlan, usize) {
    let workflow_blockers = plan
        .issues
        .iter()
        .filter(|issue| {
            issue.severity == actions_environments::IssueSeverity::Blocker
                && issue.scope.starts_with("actions.workflows.")
        })
        .count();
    let deferred = plan.workflow_state_changes.len() + workflow_blockers;
    let mut safe = plan.clone();
    safe.workflow_state_changes.clear();
    safe.issues
        .retain(|issue| !issue.scope.starts_with("actions.workflows."));
    (safe, deferred)
}

fn actions_plan_for_apply(
    plan: &actions_environments::ActionsPlan,
    config_pr_pending: bool,
) -> (actions_environments::ActionsPlan, usize) {
    if config_pr_pending {
        actions_safe_subset(plan)
    } else {
        (plan.clone(), 0)
    }
}

async fn verify_actions_safe_subset(
    client: &Client,
    repo: &str,
    desired: &ActionsCategoryV2,
) -> Result<bool> {
    let current =
        actions_environments::collect_actions_category(client, repo, Some(desired)).await?;
    let remaining = actions_environments::plan_actions_category(desired, &current);
    let (safe, _) = actions_safe_subset(&remaining);
    Ok(!safe.has_actionable_changes()
        && !safe
            .issues
            .iter()
            .any(|issue| issue.severity == actions_environments::IssueSeverity::Blocker))
}

/// Clone an integrations plan and clear the Pages action so the safe subset can
/// be applied while Pages changes are deferred. Returns the safe plan and the
/// deferred count.
fn integrations_safe_subset(
    plan: &access_integrations::IntegrationsPlan,
) -> (access_integrations::IntegrationsPlan, usize) {
    let deferred = usize::from(plan.pages_action.is_some());
    let mut safe = plan.clone();
    safe.pages_action = None;
    (safe, deferred)
}

fn integrations_plan_for_apply(
    plan: &access_integrations::IntegrationsPlan,
    config_pr_pending: bool,
) -> (access_integrations::IntegrationsPlan, usize) {
    if config_pr_pending {
        integrations_safe_subset(plan)
    } else {
        (plan.clone(), 0)
    }
}

async fn verify_integrations_safe_subset(
    client: &Client,
    repo: &str,
    desired: &RepositoryIntegrationsCategoryV2,
) -> Result<bool> {
    let current = access_integrations::collect_integrations(client, repo, desired).await?;
    let remaining = access_integrations::plan_integrations(&current, desired);
    let (safe, _) = integrations_safe_subset(&remaining);
    Ok(safe.is_empty()
        && !safe.issues.iter().any(|issue| {
            issue.severity == actions_environments::IssueSeverity::Blocker
                && !issue.scope.starts_with("integrations.pages")
        }))
}

fn dependency_deferral(category: &CategoryPlan, config_pr_pending: bool) -> DependencyDeferral {
    if !config_pr_pending || category.disposition != ManagementDisposition::Managed {
        return DependencyDeferral::default();
    }

    match &category.kind {
        CategoryPlanKind::Actions(plan) => {
            let blockers = plan
                .issues
                .iter()
                .filter(|issue| {
                    issue.severity == actions_environments::IssueSeverity::Blocker
                        && issue.scope.starts_with("actions.workflows.")
                })
                .count();
            let actionable = plan.workflow_state_changes.len();
            DependencyDeferral {
                actionable,
                blockers,
                total: actionable + blockers,
            }
        }
        CategoryPlanKind::Integrations(plan) => {
            let actionable = usize::from(plan.pages_action.is_some());
            DependencyDeferral {
                actionable,
                blockers: 0,
                total: actionable,
            }
        }
        CategoryPlanKind::Rulesets(_) | CategoryPlanKind::BranchProtection(_) => {
            DependencyDeferral {
                actionable: category.actionable,
                blockers: 0,
                total: category.actionable,
            }
        }
        _ => DependencyDeferral::default(),
    }
}

fn adjust_for_config_pr(
    mut report: CategoryReport,
    category: &CategoryPlan,
    config_pr_pending: bool,
    planning: bool,
) -> CategoryReport {
    let deferral = dependency_deferral(category, config_pr_pending);
    if deferral.total == 0 {
        return report;
    }

    report.actionable = report.actionable.saturating_sub(deferral.actionable);
    report.blocked = report.blocked.saturating_sub(deferral.blockers);
    if report.status == "blocked" && report.error.is_some() {
        report.blocked = report.blocked.max(1);
    }
    report.deferred = report.deferred.max(deferral.total);

    if !report
        .details
        .iter()
        .any(|detail| detail.contains("configuration PR"))
    {
        report.details.push(format!(
            "{} change(s) deferred until the configuration PR merges",
            deferral.total
        ));
    }

    if planning {
        report.status = if report.blocked > 0 {
            "blocked"
        } else if report.actionable > 0 {
            "planned"
        } else {
            "deferred"
        }
        .to_owned();
    } else if report.blocked == 0
        && report.actionable == 0
        && !matches!(report.status.as_str(), "failed" | "observed" | "skipped")
    {
        report.status = "deferred".to_owned();
    }

    report
}

fn blocked_with_message(
    audit: &AuditLog,
    repo: &str,
    category: &CategoryPlan,
    message: String,
) -> CategoryReport {
    audit_category(audit, repo, category, "blocked", 0, Some(message.clone()));
    let mut report = category.to_report("blocked", 0, None);
    report.error = Some(message);
    report.blocked = report.blocked.max(1);
    report
}

fn failure(
    audit: &AuditLog,
    repo: &str,
    category: &CategoryPlan,
    error: anyhow::Error,
) -> CategoryReport {
    let message = format!("{error:#}");
    audit_category(audit, repo, category, "failed", 0, Some(message.clone()));
    let mut report = category.to_report("failed", 0, None);
    report.error = Some(message);
    report
}

fn audit_category(
    audit: &AuditLog,
    repo: &str,
    category: &CategoryPlan,
    status: &str,
    deferred: usize,
    error: Option<String>,
) {
    let before = serde_json::json!({
        "category": category.name.stable_name(),
        "disposition": category.disposition_label(),
        "actionable": category.actionable,
        "blocked": category.blocked,
    });
    let mut after = serde_json::json!({
        "status": status,
        "deferred": deferred,
    });
    if let Some(message) = error {
        after["error"] = serde_json::Value::String(message);
    }
    if let Err(error) = audit.log_values(
        repo,
        &format!("apply.{}", category.name.stable_name()),
        status,
        before,
        after,
    ) {
        tracing::warn!(%error, repo, category = category.name.stable_name(), "Failed to write Ward audit entry");
    }
}

// ---------------------------------------------------------------------------
// Legacy path safety guard
// ---------------------------------------------------------------------------

/// Refuse a legacy category-specific mutation when the v2 manifest marks that
/// category as observe/reference/placeholder, so unsafe legacy defaults can
/// never bypass v2 policy. Managed v2 categories still direct users to the
/// unified command for consistent, ordered, verified application.
pub fn guard_legacy_mutation(
    manifest: &Manifest,
    category: Category,
    legacy_command: &str,
) -> Result<()> {
    // Only guard v2 manifests. Pre-v2 manifests keep their legacy behaviour
    // untouched, while hand-written v2 categories are honored even when the
    // optional schema marker was omitted.
    if manifest.v2_schema().is_none() && manifest.v2_categories().is_empty() {
        return Ok(());
    }
    let Some(policy) = category_policy(manifest, category) else {
        return Ok(());
    };
    if policy.disposition != ManagementDisposition::Managed {
        bail!(
            "Category `{name}` is `{disposition}` in this v2 manifest; `ward {legacy_command}` must not mutate it. Use `ward apply --category {name}`.",
            name = category.stable_name(),
            disposition = disposition_label(policy.disposition),
        );
    }
    Ok(())
}

fn category_policy(manifest: &Manifest, category: Category) -> Option<CategoryPolicy> {
    let categories = manifest.v2_categories();
    let policy = match category {
        Category::Repository => categories.repository.as_ref().map(policy_of_repository)?,
        Category::Files => categories.files.as_ref().map(policy_of_files)?,
        Category::Security => categories.security.as_ref().map(policy_of_security)?,
        Category::Rulesets => categories.rulesets.as_ref().map(policy_of_rulesets)?,
        Category::BranchProtection => categories
            .branch_protection
            .as_ref()
            .map(policy_of_branch_protection)?,
        Category::Actions => categories.actions.as_ref().map(policy_of_actions)?,
        Category::Environments => categories
            .environments
            .as_ref()
            .map(policy_of_environments)?,
        Category::Access => categories.access.as_ref().map(policy_of_access)?,
        Category::Integrations => categories
            .integrations
            .as_ref()
            .map(policy_of_integrations)?,
    };
    Some(policy)
}

fn policy_of_repository(category: &RepositoryCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_files(category: &FilesCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_security(category: &crate::config::manifest::SecurityCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_rulesets(category: &RulesetsCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_branch_protection(category: &BranchProtectionCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_actions(category: &ActionsCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_environments(category: &EnvironmentsCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_access(category: &RepositoryAccessCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}
fn policy_of_integrations(category: &RepositoryIntegrationsCategoryV2) -> CategoryPolicy {
    category.policy.clone()
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

const DETAIL_LIMIT: usize = 8;

fn sync_branch(manifest: &Manifest) -> String {
    let branch = manifest.file_delivery.branch.trim();
    if branch.is_empty() {
        "chore/ward-sync".to_owned()
    } else {
        branch.to_owned()
    }
}

fn commit_prefix(manifest: &Manifest) -> String {
    let prefix = manifest.file_delivery.commit_message_prefix.trim_end();
    if prefix.is_empty() {
        "chore: ".to_owned()
    } else {
        format!("{prefix} ")
    }
}

fn disposition_label(disposition: ManagementDisposition) -> String {
    match disposition {
        ManagementDisposition::Managed => "managed",
        ManagementDisposition::Reference => "reference",
        ManagementDisposition::Placeholder => "placeholder",
        ManagementDisposition::Observe => "observe",
    }
    .to_owned()
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn degraded_coverage(coverage: &[CoverageEntry]) -> usize {
    coverage
        .iter()
        .filter(|entry| {
            matches!(
                entry.outcome,
                CoverageOutcome::PermissionDenied
                    | CoverageOutcome::Unavailable
                    | CoverageOutcome::Redacted
                    | CoverageOutcome::Unsupported
            )
        })
        .count()
}

fn coverage_counts(coverage: &[CoverageEntry]) -> CoverageCounts {
    let mut counts = CoverageCounts {
        total: coverage.len(),
        ..CoverageCounts::default()
    };
    for entry in coverage {
        match entry.outcome {
            CoverageOutcome::Collected => counts.collected += 1,
            CoverageOutcome::NotApplicable => counts.not_applicable += 1,
            CoverageOutcome::PermissionDenied
            | CoverageOutcome::Unavailable
            | CoverageOutcome::Redacted
            | CoverageOutcome::Unsupported => counts.degraded += 1,
        }
    }
    counts
}

fn coverage_outcomes(coverage: &[CoverageEntry]) -> Vec<CoverageOutcomeCount> {
    use std::collections::BTreeMap;
    let mut tally: BTreeMap<&'static str, usize> = BTreeMap::new();
    for entry in coverage {
        *tally.entry(outcome_name(entry.outcome)).or_default() += 1;
    }
    tally
        .into_iter()
        .map(|(outcome, count)| CoverageOutcomeCount {
            outcome: outcome.to_owned(),
            count,
        })
        .collect()
}

fn outcome_name(outcome: CoverageOutcome) -> &'static str {
    match outcome {
        CoverageOutcome::Collected => "collected",
        CoverageOutcome::Redacted => "redacted",
        CoverageOutcome::PermissionDenied => "permission_denied",
        CoverageOutcome::Unsupported => "unsupported",
        CoverageOutcome::Unavailable => "unavailable",
        CoverageOutcome::NotApplicable => "not_applicable",
    }
}

fn reconcile_issue_counts(issues: &[security_rules::ReconcileIssue]) -> (usize, usize) {
    let blocked = issues
        .iter()
        .filter(|issue| issue.severity == security_rules::ReconcileIssueSeverity::Blocker)
        .count();
    let warnings = issues.len() - blocked;
    (blocked, warnings)
}

fn actions_issue_counts(issues: &[actions_environments::ReconcileIssue]) -> (usize, usize) {
    let blocked = issues
        .iter()
        .filter(|issue| issue.severity == actions_environments::IssueSeverity::Blocker)
        .count();
    let warnings = issues.len() - blocked;
    (blocked, warnings)
}

fn actions_actionable_count(plan: &actions_environments::ActionsPlan) -> usize {
    plan.settings_changes.len()
        + plan.workflow_state_changes.len()
        + plan.variable_upserts.len()
        + plan.variable_deletions.len()
        + plan.secret_upserts.len()
        + plan.secret_deletions.len()
        + plan.reference_actions.len()
}

fn environments_actionable_count(plan: &actions_environments::EnvironmentsPlan) -> usize {
    plan.environment_deletions.len()
        + plan
            .environment_plans
            .iter()
            .filter(|env| env.has_actionable_changes())
            .count()
}

fn describe_general_change(change: &general::GeneralChange) -> String {
    use general::GeneralChangeKind;
    match &change.kind {
        GeneralChangeKind::RestField { field } | GeneralChangeKind::GraphqlField { field } => {
            format!("{field}: {} -> {}", change.current, change.desired)
        }
        GeneralChangeKind::Topics => format!("topics: {} -> {}", change.current, change.desired),
        GeneralChangeKind::CustomProperty { property_name, .. } => {
            format!("custom property {property_name}")
        }
        GeneralChangeKind::ImmutableReleases { .. } => "immutable releases".to_owned(),
        GeneralChangeKind::Label { name, .. } => format!("label {name}"),
    }
}

fn security_change_details(plan: &security_rules::SecurityPlan) -> Vec<String> {
    let mut details = Vec::new();
    if plan.dependabot_alerts.is_some() {
        details.push("dependabot alerts".to_owned());
    }
    if plan.dependabot_security_updates.is_some() {
        details.push("dependabot security updates".to_owned());
    }
    if plan.private_vulnerability_reporting.is_some() {
        details.push("private vulnerability reporting".to_owned());
    }
    if plan.codeql_default_setup.is_some() {
        details.push("codeql default setup".to_owned());
    }
    if plan.patch_security_and_analysis.is_some() {
        details.push("security and analysis".to_owned());
    }
    if plan.attach_configuration_id.is_some() {
        details.push("attach code security configuration".to_owned());
    }
    if plan.detach_configuration {
        details.push("detach code security configuration".to_owned());
    }
    for issue in plan.issues.iter().take(DETAIL_LIMIT) {
        details.push(format!("{:?}: {}", issue.severity, issue.message));
    }
    details
}

fn ruleset_action_details(plan: &security_rules::RulesetsPlan) -> Vec<String> {
    let mut details = Vec::new();
    for action in plan.actions.iter().take(DETAIL_LIMIT) {
        match action {
            security_rules::RulesetPlanAction::Create { ruleset } => {
                details.push(format!("create ruleset {}", ruleset.name))
            }
            security_rules::RulesetPlanAction::Update { ruleset, .. } => {
                details.push(format!("update ruleset {}", ruleset.name))
            }
            security_rules::RulesetPlanAction::Delete { name, .. } => {
                details.push(format!("delete ruleset {name}"))
            }
            security_rules::RulesetPlanAction::Unchanged { .. } => {}
        }
    }
    for issue in plan.issues.iter().take(DETAIL_LIMIT) {
        details.push(format!("{:?}: {}", issue.severity, issue.message));
    }
    details
}

fn branch_protection_details(plan: &security_rules::BranchProtectionPlan) -> Vec<String> {
    let mut details = Vec::new();
    for action in plan.actions.iter().take(DETAIL_LIMIT) {
        match action {
            security_rules::BranchProtectionPlanAction::Upsert { branch, .. } => {
                details.push(format!("protect branch {branch}"))
            }
            security_rules::BranchProtectionPlanAction::Delete { branch } => {
                details.push(format!("remove protection {branch}"))
            }
            security_rules::BranchProtectionPlanAction::Unchanged { .. } => {}
        }
    }
    for issue in plan.issues.iter().take(DETAIL_LIMIT) {
        details.push(format!("{:?}: {}", issue.severity, issue.message));
    }
    details
}

fn actions_details(plan: &actions_environments::ActionsPlan) -> Vec<String> {
    let mut details = Vec::new();
    if !plan.settings_changes.is_empty() {
        details.push(format!(
            "{} settings change(s)",
            plan.settings_changes.len()
        ));
    }
    if !plan.workflow_state_changes.is_empty() {
        details.push(format!(
            "{} workflow toggle(s)",
            plan.workflow_state_changes.len()
        ));
    }
    if !plan.variable_upserts.is_empty() || !plan.variable_deletions.is_empty() {
        details.push(format!(
            "{} variable change(s)",
            plan.variable_upserts.len() + plan.variable_deletions.len()
        ));
    }
    if !plan.secret_upserts.is_empty() || !plan.secret_deletions.is_empty() {
        details.push(format!(
            "{} secret change(s)",
            plan.secret_upserts.len() + plan.secret_deletions.len()
        ));
    }
    for issue in plan.issues.iter().take(DETAIL_LIMIT) {
        details.push(format!("{:?}: {}", issue.severity, issue.message));
    }
    details
}

// ---------------------------------------------------------------------------
// Human-readable rendering (shared by plan and apply)
// ---------------------------------------------------------------------------

/// Render a concise, human-readable summary of a unified report.
pub fn render_report(report: &UnifiedReport, title: &str) {
    use console::style;

    println!();
    println!("  {}", style(title).bold().cyan());

    for repo in &report.repos {
        println!();
        println!("  {}", style(&repo.repo).bold());
        for category in &repo.categories {
            if category.status == "skipped" {
                continue;
            }
            let status = style_status(&category.status);
            println!(
                "    {:<18} {:<9} {status}  actionable={} blocked={} warnings={} deferred={}",
                category.category,
                category.disposition,
                category.actionable,
                category.blocked,
                category.warnings,
                category.deferred,
            );
            for detail in category.details.iter().take(4) {
                println!("        - {detail}");
            }
        }
    }

    println!();
    println!(
        "  Summary: {} actionable, {} blocked, {} deferred, {} warnings across {} repo(s)",
        style(report.actionable).bold(),
        style(report.blocked).bold(),
        style(report.deferred).bold(),
        style(report.warnings).bold(),
        report.repos.len(),
    );
    println!(
        "  Coverage: {}/{} collected, {} degraded",
        report.coverage.collected, report.coverage.total, report.coverage.degraded,
    );
}

fn style_status(status: &str) -> console::StyledObject<&str> {
    use console::style;
    match status {
        "success" => style(status).green(),
        "noop" | "observed" => style(status).dim(),
        "deferred" => style(status).yellow(),
        "planned" => style(status).cyan(),
        "blocked" | "failed" => style(status).red(),
        _ => style(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stable_category_names() {
        assert_eq!(Category::parse("repository").unwrap(), Category::Repository);
        assert_eq!(
            Category::parse("branch-protection").unwrap(),
            Category::BranchProtection
        );
        assert_eq!(
            Category::parse("integrations").unwrap(),
            Category::Integrations
        );
        assert!(Category::parse("nope").is_err());
    }

    #[test]
    fn empty_selection_is_all_categories() {
        let all = parse_categories(&[]).unwrap();
        assert_eq!(all.len(), 9);
        assert!(all.contains(&Category::Files));
    }

    #[test]
    fn category_selection_deduplicates() {
        let selected = parse_categories(&["security".to_owned(), "security".to_owned()]).unwrap();
        assert_eq!(selected, vec![Category::Security]);
    }

    #[test]
    fn coverage_counts_split_by_outcome() {
        use crate::config::manifest::ManifestCategoryName;
        let coverage = vec![
            CoverageEntry {
                category: ManifestCategoryName::Security,
                endpoint: "a".to_owned(),
                outcome: CoverageOutcome::Collected,
                reason: None,
                required_permission: None,
            },
            CoverageEntry {
                category: ManifestCategoryName::Security,
                endpoint: "b".to_owned(),
                outcome: CoverageOutcome::PermissionDenied,
                reason: None,
                required_permission: None,
            },
        ];
        let counts = coverage_counts(&coverage);
        assert_eq!(counts.total, 2);
        assert_eq!(counts.collected, 1);
        assert_eq!(counts.degraded, 1);
        assert_eq!(degraded_coverage(&coverage), 1);
    }

    fn empty_files_plan() -> files::FilesPlan {
        files::FilesPlan {
            upserts: Vec::new(),
            deletions: Vec::new(),
            unchanged: Vec::new(),
            atomic_entries: Vec::new(),
            issues: Vec::new(),
        }
    }

    fn planned_category(
        name: Category,
        disposition: ManagementDisposition,
        actionable: usize,
        blocked: usize,
    ) -> CategoryPlan {
        CategoryPlan {
            name,
            disposition,
            is_blocked_collection: false,
            coverage: Vec::new(),
            actionable,
            blocked,
            warnings: 0,
            details: Vec::new(),
            kind: CategoryPlanKind::Files(empty_files_plan()),
        }
    }

    #[test]
    fn plan_status_reflects_disposition_and_counts() {
        assert_eq!(
            planned_category(Category::Files, ManagementDisposition::Managed, 3, 0).plan_status(),
            "planned"
        );
        assert_eq!(
            planned_category(Category::Files, ManagementDisposition::Managed, 0, 0).plan_status(),
            "noop"
        );
        assert_eq!(
            planned_category(Category::Files, ManagementDisposition::Observe, 0, 0).plan_status(),
            "observed"
        );
        assert_eq!(
            planned_category(Category::Files, ManagementDisposition::Managed, 2, 1).plan_status(),
            "blocked"
        );
        assert_eq!(absent_category(Category::Files).plan_status(), "skipped");
        assert_eq!(
            collection_failed(
                Category::Files,
                ManagementDisposition::Managed,
                anyhow::anyhow!("boom"),
            )
            .plan_status(),
            "blocked"
        );
    }

    #[test]
    fn observe_category_reports_zero_actions_and_no_error() {
        let category = planned_category(Category::Access, ManagementDisposition::Observe, 0, 0);
        let report = category.to_report(category.plan_status(), 0, None);
        assert_eq!(report.status, "observed");
        assert_eq!(report.actionable, 0);
        assert_eq!(report.disposition, "observe");
        assert!(report.error.is_none());
    }

    #[test]
    fn collection_failure_is_surfaced_not_hidden() {
        let category = collection_failed(
            Category::Security,
            ManagementDisposition::Managed,
            anyhow::anyhow!("HTTP 500"),
        );
        let report = category.to_report("blocked", 0, None);
        assert_eq!(report.status, "blocked");
        assert_eq!(report.blocked, 1);
        assert_eq!(report.disposition, "blocked");
        assert!(report.error.as_deref().unwrap().contains("HTTP 500"));
    }

    #[test]
    fn partial_collector_failure_keeps_other_categories() {
        // One category fails collection, an unrelated one still plans.
        let plan = RepoPlan {
            repo: "repo-a".to_owned(),
            default_branch: "main".to_owned(),
            categories: vec![
                collection_failed(
                    Category::Security,
                    ManagementDisposition::Managed,
                    anyhow::anyhow!("collect failed"),
                ),
                planned_category(Category::Files, ManagementDisposition::Managed, 1, 0),
            ],
        };
        let report = plan.to_report();
        assert_eq!(report.categories.len(), 2);
        let security = &report.categories[0];
        let files = &report.categories[1];
        assert_eq!(security.status, "blocked");
        assert!(security.error.is_some());
        assert_eq!(files.status, "planned");
        assert_eq!(files.actionable, 1);
        assert_eq!(report.blocked, 1);
        assert_eq!(report.actionable, 1);
    }

    #[test]
    fn unified_report_aggregates_and_detects_failures() {
        let ok =
            aggregate_repo(
                "repo-ok".to_owned(),
                vec![
                    planned_category(Category::Files, ManagementDisposition::Managed, 2, 0)
                        .to_report("success", 0, Some(true)),
                ],
            );
        let bad = aggregate_repo(
            "repo-bad".to_owned(),
            vec![
                planned_category(Category::Security, ManagementDisposition::Managed, 1, 1)
                    .to_report("blocked", 0, None),
            ],
        );
        let report = UnifiedReport::from_repos(vec![ok, bad]);
        assert_eq!(report.actionable, 3);
        assert_eq!(report.blocked, 1);
        assert!(report.has_failures());

        let clean = UnifiedReport::from_repos(vec![aggregate_repo(
            "repo".to_owned(),
            vec![
                planned_category(Category::Files, ManagementDisposition::Managed, 0, 0)
                    .to_report("noop", 0, None),
            ],
        )]);
        assert!(!clean.has_failures());
    }

    #[test]
    fn json_summary_shape_is_stable() {
        let report = UnifiedReport::from_repos(vec![aggregate_repo(
            "repo-a".to_owned(),
            vec![
                planned_category(Category::Repository, ManagementDisposition::Managed, 1, 0)
                    .to_report("planned", 0, None),
            ],
        )]);
        let value = serde_json::to_value(&report).unwrap();
        assert!(value.get("repos").is_some());
        assert_eq!(value["actionable"], 1);
        assert_eq!(value["blocked"], 0);
        assert_eq!(value["deferred"], 0);
        assert!(value.get("warnings").is_some());
        assert!(value.get("coverage").is_some());
        let category = &value["repos"][0]["categories"][0];
        assert_eq!(category["category"], "repository");
        assert_eq!(category["disposition"], "managed");
        assert_eq!(category["status"], "planned");
        assert!(category.get("coverage").is_some());
        assert!(category.get("coverage_outcomes").is_some());
        assert!(category.get("details").is_some());
    }

    #[test]
    fn actions_safe_subset_defers_workflow_toggles() {
        let plan = actions_environments::ActionsPlan {
            settings_changes: Vec::new(),
            workflow_state_changes: vec![actions_environments::WorkflowStateChange {
                path: ".github/workflows/ci.yml".to_owned(),
                enabled: true,
            }],
            variable_upserts: Vec::new(),
            variable_deletions: Vec::new(),
            secret_upserts: Vec::new(),
            secret_deletions: Vec::new(),
            reference_actions: Vec::new(),
            issues: vec![actions_environments::ReconcileIssue {
                scope: "actions.workflows..github/workflows/missing.yml".to_owned(),
                severity: actions_environments::IssueSeverity::Blocker,
                message: "missing".to_owned(),
            }],
        };
        let (safe, deferred) = actions_safe_subset(&plan);
        assert_eq!(deferred, 2);
        assert!(safe.workflow_state_changes.is_empty());
        assert!(safe.issues.is_empty());

        let (direct, deferred) = actions_plan_for_apply(&plan, false);
        assert_eq!(deferred, 0);
        assert_eq!(direct.workflow_state_changes.len(), 1);
        assert_eq!(direct.issues.len(), 1);
    }

    #[test]
    fn planned_file_pr_reclassifies_missing_workflow_as_deferred() {
        let actions = CategoryPlan {
            name: Category::Actions,
            disposition: ManagementDisposition::Managed,
            is_blocked_collection: false,
            coverage: Vec::new(),
            actionable: 0,
            blocked: 1,
            warnings: 0,
            details: Vec::new(),
            kind: CategoryPlanKind::Actions(actions_environments::ActionsPlan {
                settings_changes: Vec::new(),
                workflow_state_changes: Vec::new(),
                variable_upserts: Vec::new(),
                variable_deletions: Vec::new(),
                secret_upserts: Vec::new(),
                secret_deletions: Vec::new(),
                reference_actions: Vec::new(),
                issues: vec![actions_environments::ReconcileIssue {
                    scope: "actions.workflows..github/workflows/ci.yml".to_owned(),
                    severity: actions_environments::IssueSeverity::Blocker,
                    message: "Workflow file not found".to_owned(),
                }],
            }),
        };
        let plan = RepoPlan {
            repo: "repo".to_owned(),
            default_branch: "main".to_owned(),
            categories: vec![
                planned_category(Category::Files, ManagementDisposition::Managed, 1, 0),
                actions,
            ],
        };

        let report = plan.to_report();
        let actions = &report.categories[1];
        assert_eq!(actions.status, "deferred");
        assert_eq!(actions.blocked, 0);
        assert_eq!(actions.deferred, 1);
        assert_eq!(report.blocked, 0);
    }

    #[test]
    fn integrations_safe_subset_defers_pages() {
        let plan = access_integrations::IntegrationsPlan {
            policy: CategoryPolicy::managed(),
            webhook_actions: Vec::new(),
            deploy_key_actions: Vec::new(),
            pages_action: Some(access_integrations::PagesAction::Delete),
            autolink_actions: Vec::new(),
            notes: Vec::new(),
            issues: Vec::new(),
        };
        let (safe, deferred) = integrations_safe_subset(&plan);
        assert_eq!(deferred, 1);
        assert!(safe.pages_action.is_none());

        let (direct, deferred) = integrations_plan_for_apply(&plan, false);
        assert_eq!(deferred, 0);
        assert!(direct.pages_action.is_some());
    }

    #[test]
    fn files_verification_mismatch_is_a_failed_report() {
        let category = planned_category(Category::Files, ManagementDisposition::Managed, 1, 0);
        let report = files_apply_report(
            &category,
            vec!["PR branch does not match desired state after commit".to_owned()],
            false,
        );

        assert_eq!(report.status, "failed");
        assert_eq!(report.verified, Some(false));
        assert!(report.configuration_pull_request_pending);
        assert!(report.details[0].contains("does not match"));
    }

    #[test]
    fn rulesets_and_branch_protection_defer_behind_config_pr() {
        let rulesets = CategoryPlan {
            name: Category::Rulesets,
            disposition: ManagementDisposition::Managed,
            is_blocked_collection: false,
            coverage: Vec::new(),
            actionable: 3,
            blocked: 0,
            warnings: 0,
            details: Vec::new(),
            kind: CategoryPlanKind::Rulesets(security_rules::RulesetsPlan {
                actions: Vec::new(),
                issues: Vec::new(),
            }),
        };
        let branch_protection = CategoryPlan {
            name: Category::BranchProtection,
            disposition: ManagementDisposition::Managed,
            is_blocked_collection: false,
            coverage: Vec::new(),
            actionable: 2,
            blocked: 0,
            warnings: 0,
            details: Vec::new(),
            kind: CategoryPlanKind::BranchProtection(security_rules::BranchProtectionPlan {
                actions: Vec::new(),
                issues: Vec::new(),
            }),
        };

        assert_eq!(dependency_deferral(&rulesets, true).total, 3);
        assert_eq!(dependency_deferral(&branch_protection, true).total, 2);
        assert_eq!(dependency_deferral(&rulesets, false).total, 0);
    }

    #[test]
    fn build_general_desired_merges_integration_labels_under_repository_policy() {
        use crate::config::manifest::{
            LabelConfigV2, ManifestSchema, RepositoryCategoryV2, RepositoryIntegrationsCategoryV2,
        };

        let mut manifest = Manifest::default();
        manifest.v2.schema = Some(ManifestSchema::v2());
        manifest.v2.categories.repository = Some(RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: None,
            metadata: None,
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        });
        manifest.v2.categories.integrations = Some(RepositoryIntegrationsCategoryV2 {
            policy: CategoryPolicy::observe_sensitive(),
            labels: vec![LabelConfigV2 {
                name: "bug".to_owned(),
                color: Some("d73a4a".to_owned()),
                description: Some("Something is broken".to_owned()),
                default: Some(true),
            }],
            ..RepositoryIntegrationsCategoryV2::default()
        });

        let desired = build_general_desired(&manifest).unwrap();
        assert_eq!(desired.labels.len(), 1);
        assert_eq!(desired.labels[0].name, "bug");
        // Repository policy governs labels: managed here.
        assert_eq!(
            desired.repository.policy.disposition,
            ManagementDisposition::Managed
        );
    }

    #[test]
    fn guard_allows_pre_v2_manifests() {
        let manifest = Manifest::default();
        assert!(guard_legacy_mutation(&manifest, Category::Security, "security apply").is_ok());
    }

    #[test]
    fn guard_refuses_observe_category_in_v2_manifest() {
        use crate::config::manifest::{ManifestSchema, RepositoryAccessCategoryV2};
        let mut manifest = Manifest::default();
        manifest.v2.schema = Some(ManifestSchema::v2());
        manifest.v2.categories.access = Some(RepositoryAccessCategoryV2::observe_sensitive());

        let err = guard_legacy_mutation(&manifest, Category::Access, "teams apply").unwrap_err();
        let message = format!("{err}");
        assert!(message.contains("ward apply"));
        assert!(message.contains("access"));
    }

    #[test]
    fn guard_refuses_v2_category_without_schema_marker() {
        use crate::config::manifest::RepositoryAccessCategoryV2;
        let mut manifest = Manifest::default();
        manifest.v2.categories.access = Some(RepositoryAccessCategoryV2::observe_sensitive());

        assert!(guard_legacy_mutation(&manifest, Category::Access, "teams apply").is_err());
    }

    #[test]
    fn guard_allows_managed_category_in_v2_manifest() {
        use crate::config::manifest::{ManifestSchema, SecurityCategoryV2};
        let mut manifest = Manifest::default();
        manifest.v2.schema = Some(ManifestSchema::v2());
        let mut security = SecurityCategoryV2::observe_sensitive();
        security.policy = CategoryPolicy::managed();
        manifest.v2.categories.security = Some(security);

        assert!(guard_legacy_mutation(&manifest, Category::Security, "security apply").is_ok());
    }

    #[test]
    fn apply_order_covers_all_categories_repository_first_protection_last() {
        let order = Category::apply_order();
        assert_eq!(order.len(), 9);
        assert_eq!(order[0], Category::Repository);
        assert_eq!(order[1], Category::Files);
        assert_eq!(order[8], Category::BranchProtection);
    }
}
