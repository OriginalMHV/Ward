use anyhow::Result;

use crate::config::Manifest;
use crate::config::manifest::{
    CoverageEntry, CoverageOutcome, ManagementDisposition, ManifestCategories, RepositoryCategory,
};
use crate::github::Client;
use crate::github::repos::Repository;
use crate::reconcile::{access_integrations, actions_environments, files, general, security_rules};

use super::gating::sync_branch;
use super::model::{
    CategoryPlan, CategoryPlanKind, DETAIL_LIMIT, RepoPlan, actions_actionable_count,
    actions_details, actions_issue_counts, branch_protection_details, degraded_coverage,
    describe_general_change, environments_actionable_count, reconcile_issue_counts,
    ruleset_action_details, security_change_details,
};
use super::report::UnifiedReport;
use super::{Category, UnifiedOptions};

// ---------------------------------------------------------------------------
// Desired-state assembly
// ---------------------------------------------------------------------------

/// Build the combined general/repository desired state: the manifest
/// repository category plus labels stored in the integrations category. The
/// repository category policy governs whether those labels are managed.
pub(super) fn build_general_desired(
    categories: &ManifestCategories,
) -> Option<general::GeneralDesiredState> {
    let repository = categories.repository.clone()?;
    let mut desired = general::GeneralDesiredState::from(repository);
    if let Some(integrations) = categories.integrations.as_ref() {
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
    let branch = sync_branch(manifest);
    let repo_reports = crate::reconcile::map_buffered(repos, |repository| async {
        let mut plan = plan_repo(client, manifest, repository, options).await;
        let existing_config_pr =
            if !plan.plans_config_pull_request() && plan.has_file_dependent_changes() {
                match client.find_open_pull_request(&plan.repo, &branch).await {
                    Ok(pull_request) => pull_request.is_some(),
                    Err(error) => {
                        plan.record_config_pr_lookup_failure(&format!("{error:#}"));
                        false
                    }
                }
            } else {
                false
            };
        plan.to_report_with_config_pr(existing_config_pr)
    })
    .await;
    Ok(UnifiedReport::from_repos(repo_reports))
}

async fn plan_repo(
    client: &Client,
    manifest: &Manifest,
    repository: &Repository,
    options: &UnifiedOptions,
) -> RepoPlan {
    let repo = repository.name.clone();
    let desired_categories = manifest.categories_for_repo(&repo);
    let selected: Vec<Category> = Category::apply_order()
        .into_iter()
        .filter(|category| options.includes(*category))
        .collect();

    // The repository and security categories both read `GET /repos/{repo}`.
    // Fetch it once. On failure each category falls back to its own read.
    let needs_repo_read = (selected.contains(&Category::Repository)
        && desired_categories.repository.is_some())
        || (selected.contains(&Category::Security) && desired_categories.security.is_some());
    let repo_data = if needs_repo_read {
        client.get_repo_value(&repo).await.ok()
    } else {
        None
    };
    let shared = SharedRepoData {
        general: repo_data
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        security: repo_data
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        default_branch: &repository.default_branch,
    };

    let categories = futures_util::future::join_all(selected.iter().map(|category| {
        plan_category(
            client,
            &desired_categories,
            &repo,
            *category,
            options,
            &shared,
        )
    }))
    .await;

    RepoPlan {
        repo,
        default_branch: repository.default_branch.clone(),
        categories,
    }
}

/// Views of one repository fetch, shared by the categories planned for it.
struct SharedRepoData<'a> {
    general: Option<crate::github::settings::RepositoryGeneralSettings>,
    security: Option<crate::github::security::RepositorySecurityBaseline>,
    default_branch: &'a str,
}

async fn plan_category(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
    category: Category,
    options: &UnifiedOptions,
    shared: &SharedRepoData<'_>,
) -> CategoryPlan {
    match category {
        Category::Repository => {
            plan_repository(client, categories, repo, options, shared.general.clone()).await
        }
        Category::Files => plan_files(client, categories, repo).await,
        Category::Security => {
            plan_security(client, categories, repo, shared.security.clone()).await
        }
        Category::Rulesets => plan_rulesets(client, categories, repo).await,
        Category::BranchProtection => {
            plan_branch_protection(client, categories, repo, shared.default_branch).await
        }
        Category::Actions => plan_actions(client, categories, repo).await,
        Category::Environments => plan_environments(client, categories, repo).await,
        Category::Access => plan_access(client, categories, repo).await,
        Category::Integrations => plan_integrations(client, categories, repo).await,
    }
}

pub(super) fn absent_category(name: Category) -> CategoryPlan {
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

pub(super) fn collection_failed(
    name: Category,
    disposition: ManagementDisposition,
    error: &anyhow::Error,
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

/// Custom properties and immutable releases are optional reads. A failure on them
/// is unknown managed state only when the manifest actually manages them.
pub(super) fn relax_unrequested_repository_coverage(
    coverage: &mut [CoverageEntry],
    desired: &RepositoryCategory,
) {
    let wants_properties = !desired.custom_properties.is_empty() || desired.policy.prune;
    let wants_immutable_releases = desired.immutable_releases.is_some();
    for entry in coverage {
        let requested = if entry.endpoint.ends_with("/properties/values") {
            wants_properties
        } else if entry.endpoint.ends_with("/immutable-releases") {
            wants_immutable_releases
        } else {
            continue;
        };
        if !requested
            && matches!(
                entry.outcome,
                CoverageOutcome::PermissionDenied | CoverageOutcome::Unavailable
            )
        {
            entry.outcome = CoverageOutcome::NotApplicable;
            entry.reason = Some(format!(
                "not required by the manifest: {}",
                entry.reason.take().unwrap_or_default()
            ));
        }
    }
}

async fn plan_repository(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
    options: &UnifiedOptions,
    prefetched: Option<crate::github::settings::RepositoryGeneralSettings>,
) -> CategoryPlan {
    let Some(desired) = build_general_desired(categories) else {
        return absent_category(Category::Repository);
    };
    let disposition = desired.repository.policy.disposition;
    let collected = match prefetched {
        Some(rest) => general::collect_with_rest(client, repo, rest).await,
        None => general::collect(client, repo).await,
    };
    let mut current = match collected {
        Ok(state) => state,
        Err(error) => return collection_failed(Category::Repository, disposition, &error),
    };
    relax_unrequested_repository_coverage(&mut current.coverage, &desired.repository);
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

async fn plan_files(client: &Client, categories: &ManifestCategories, repo: &str) -> CategoryPlan {
    let Some(desired) = categories.files.clone() else {
        return absent_category(Category::Files);
    };
    let disposition = desired.policy.disposition;
    let collection = match files::collect_files_category(client, repo, None, Some(&desired)).await {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::Files, disposition, &error),
    };
    let coverage = collection.coverage.clone();
    let plan = match files::plan_files_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::Files, disposition, &error),
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

async fn plan_security(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
    prefetched: Option<crate::github::security::RepositorySecurityBaseline>,
) -> CategoryPlan {
    let Some(desired) = categories.security.clone() else {
        return absent_category(Category::Security);
    };
    let disposition = desired.policy.disposition;
    let collected = match prefetched {
        Some(baseline) => {
            security_rules::collect_security_category_with_baseline(
                client,
                repo,
                baseline,
                Some(&desired),
            )
            .await
        }
        None => security_rules::collect_security_category(client, repo, Some(&desired)).await,
    };
    let collection = match collected {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::Security, disposition, &error),
    };
    let coverage = collection.coverage.clone();
    let plan = match security_rules::plan_security_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::Security, disposition, &error),
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

async fn plan_rulesets(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
) -> CategoryPlan {
    let Some(desired) = categories.rulesets.clone() else {
        return absent_category(Category::Rulesets);
    };
    let disposition = desired.policy.disposition;
    let collection =
        match security_rules::collect_rulesets_category(client, repo, Some(&desired)).await {
            Ok(collection) => collection,
            Err(error) => return collection_failed(Category::Rulesets, disposition, &error),
        };
    let coverage = collection.coverage.clone();
    let plan = match security_rules::plan_rulesets_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::Rulesets, disposition, &error),
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

async fn plan_branch_protection(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
    default_branch: &str,
) -> CategoryPlan {
    let Some(desired) = categories.branch_protection.clone() else {
        return absent_category(Category::BranchProtection);
    };
    let disposition = desired.policy.disposition;
    let collection = match security_rules::collect_branch_protection_category_for_branch(
        client,
        repo,
        default_branch.to_owned(),
        Some(&desired),
    )
    .await
    {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::BranchProtection, disposition, &error),
    };
    let coverage = collection.coverage.clone();
    let plan = match security_rules::plan_branch_protection_category(&desired, &collection) {
        Ok(plan) => plan,
        Err(error) => return collection_failed(Category::BranchProtection, disposition, &error),
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

async fn plan_actions(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
) -> CategoryPlan {
    let Some(desired) = categories.actions.clone() else {
        return absent_category(Category::Actions);
    };
    let disposition = desired.policy.disposition;
    let collection =
        match actions_environments::collect_actions_category(client, repo, Some(&desired)).await {
            Ok(collection) => collection,
            Err(error) => return collection_failed(Category::Actions, disposition, &error),
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

async fn plan_environments(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
) -> CategoryPlan {
    let Some(desired) = categories.environments.clone() else {
        return absent_category(Category::Environments);
    };
    let disposition = desired.policy.disposition;
    let collection =
        match actions_environments::collect_environments_category(client, repo, Some(&desired))
            .await
        {
            Ok(collection) => collection,
            Err(error) => return collection_failed(Category::Environments, disposition, &error),
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

async fn plan_access(client: &Client, categories: &ManifestCategories, repo: &str) -> CategoryPlan {
    let Some(desired) = categories.access.clone() else {
        return absent_category(Category::Access);
    };
    let disposition = desired.policy.disposition;
    let collection = match access_integrations::collect_access(client, repo, &desired).await {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::Access, disposition, &error),
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

pub(super) async fn plan_integrations(
    client: &Client,
    categories: &ManifestCategories,
    repo: &str,
) -> CategoryPlan {
    let Some(desired) = categories.integrations.clone() else {
        return absent_category(Category::Integrations);
    };
    let disposition = desired.policy.disposition;
    let collection = match access_integrations::collect_integrations(client, repo, &desired).await {
        Ok(collection) => collection,
        Err(error) => return collection_failed(Category::Integrations, disposition, &error),
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

/// Read-only plans for every target repository, ready to apply unchanged.
pub struct PreparedApply {
    pub(super) prepared: Vec<(RepoPlan, bool)>,
}

impl PreparedApply {
    /// The plan report for what `apply` would do.
    pub fn report(&self) -> UnifiedReport {
        UnifiedReport::from_repos(
            self.prepared
                .iter()
                .map(|(plan, existing_config_pr)| {
                    plan.to_report_with_config_pr(*existing_config_pr)
                })
                .collect(),
        )
    }
}

pub async fn prepare_apply(
    client: &Client,
    manifest: &Manifest,
    repos: &[Repository],
    options: &UnifiedOptions,
) -> Result<PreparedApply> {
    let (archived, repos): (Vec<&Repository>, Vec<&Repository>) =
        repos.iter().partition(|repository| repository.archived);
    for repository in archived {
        tracing::warn!(
            "Skipping archived repository {}. Ward plans and audits archived repositories but does not apply changes to them",
            repository.name
        );
        #[allow(
            clippy::print_stderr,
            reason = "user-visible warning; moves to cli with the module split"
        )]
        {
            eprintln!(
                "  warning: skipping archived repository {}",
                repository.name
            );
        }
    }
    let branch = sync_branch(manifest);
    let prepared = crate::reconcile::map_buffered(repos, |repository| async {
        let plan = plan_repo(client, manifest, repository, options).await;
        let existing_config_pr = if plan.has_file_dependent_changes() {
            client
                .find_open_pull_request(&plan.repo, &branch)
                .await?
                .is_some()
        } else {
            false
        };
        Ok::<_, anyhow::Error>((plan, existing_config_pr))
    })
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    Ok(PreparedApply { prepared })
}
