use anyhow::Result;

use crate::config::Manifest;
use crate::config::manifest::{ManagementDisposition, ManifestCategories};
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::github::repos::Repository;
use crate::reconcile::{access_integrations, actions_environments, files, general, security_rules};

use super::UnifiedOptions;
use super::gating::{
    actions_plan_for_apply, adjust_for_config_pr, audit_category, blocked_with_message,
    commit_prefix, dependency_deferral, failure, finish_success, finish_success_deferred,
    integrations_plan_for_apply, name_pull_request, sync_branch, verify_actions_safe_subset,
    verify_integrations_safe_subset,
};
use super::model::{CategoryPlan, CategoryPlanKind, RepoPlan, aggregate_repo};
use super::plan::{PreparedApply, prepare_apply};
use super::report::{CategoryReport, RepoReport, UnifiedReport};

/// Apply plans from [`prepare_apply`] in the safe order, verify results, and
/// emit a structured audit trail.
pub async fn apply_prepared(
    client: &Client,
    manifest: &Manifest,
    prepared: PreparedApply,
    options: &UnifiedOptions,
    audit: &AuditLog,
) -> UnifiedReport {
    let mut repo_reports = Vec::with_capacity(prepared.prepared.len());
    for (plan, existing_config_pr) in prepared.prepared {
        let report = apply_repo(
            client,
            manifest,
            plan,
            existing_config_pr,
            options.verify,
            audit,
        )
        .await;
        repo_reports.push(report);
    }
    UnifiedReport::from_repos(repo_reports)
}

/// Plan and apply every selected category for every target repository.
pub async fn apply(
    client: &Client,
    manifest: &Manifest,
    repos: &[Repository],
    options: &UnifiedOptions,
    audit: &AuditLog,
) -> Result<UnifiedReport> {
    let prepared = prepare_apply(client, manifest, repos, options).await?;
    Ok(apply_prepared(client, manifest, prepared, options, audit).await)
}

async fn apply_repo(
    client: &Client,
    manifest: &Manifest,
    plan: RepoPlan,
    existing_config_pr: bool,
    verify: bool,
    audit: &AuditLog,
) -> RepoReport {
    // Apply and post-apply verification read organization lookups fresh.
    let client = &client.uncached();
    let repo = plan.repo.clone();
    let default_branch = plan.default_branch.clone();
    let branch = sync_branch(manifest);
    let commit_prefix = commit_prefix(manifest);
    let desired_categories = manifest.categories_for_repo(&repo);

    let mut config_pr_pending = existing_config_pr;
    let mut config_pr_url: Option<String> = None;
    let mut reports = Vec::with_capacity(plan.categories.len());

    for category in plan.categories {
        let mut report = apply_category(
            client,
            manifest,
            &desired_categories,
            &repo,
            &default_branch,
            &branch,
            &commit_prefix,
            &category,
            config_pr_pending,
            verify,
            audit,
        )
        .await;

        // A config pull request is pending once file changes were routed
        // through it. Downstream categories that depend on those files must be
        // deferred until the PR merges.
        if report.configuration_pull_request_pending {
            config_pr_pending = true;
        }
        if let Some(url) = report
            .details
            .iter()
            .find_map(|detail| detail.strip_prefix("pull request: "))
        {
            config_pr_url = Some(url.to_owned());
        }
        if let Some(url) = &config_pr_url {
            name_pull_request(&mut report, url);
        }

        reports.push(report);
    }

    aggregate_repo(repo, reports)
}

#[allow(clippy::too_many_arguments)]
async fn apply_category(
    client: &Client,
    manifest: &Manifest,
    desired_categories: &ManifestCategories,
    repo: &str,
    default_branch: &str,
    branch: &str,
    commit_prefix: &str,
    category: &CategoryPlan,
    config_pr_pending: bool,
    verify: bool,
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
                desired_categories,
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
            apply_security(
                client,
                desired_categories,
                repo,
                category,
                plan,
                verify,
                audit,
            )
            .await
        }
        CategoryPlanKind::Actions(plan) => {
            apply_actions(
                client,
                desired_categories,
                repo,
                category,
                plan,
                config_pr_pending,
                audit,
            )
            .await
        }
        CategoryPlanKind::Environments(plan) => {
            apply_environments(client, desired_categories, repo, category, plan, audit).await
        }
        CategoryPlanKind::Access(plan) => {
            apply_access(client, desired_categories, repo, category, plan, audit).await
        }
        CategoryPlanKind::Integrations(plan) => {
            apply_integrations(
                client,
                desired_categories,
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
                desired_categories,
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
                desired_categories,
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
        Err(error) => failure(audit, repo, category, &error),
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_files(
    client: &Client,
    manifest: &Manifest,
    desired_categories: &ManifestCategories,
    repo: &str,
    default_branch: &str,
    branch: &str,
    commit_prefix: &str,
    category: &CategoryPlan,
    _default_branch_plan: &files::FilesPlan,
    audit: &AuditLog,
) -> CategoryReport {
    let Some(desired) = desired_categories.files.clone() else {
        return category.to_report("skipped", 0, None);
    };

    // Never write files to the default branch: route through a dedicated
    // branch and a pull request.
    if let Err(error) = client
        .ensure_dedicated_branch(repo, branch, default_branch)
        .await
    {
        return failure(audit, repo, category, &error);
    }

    // Re-collect and re-plan against the dedicated branch so we only commit
    // changes still missing there.
    let branch_collection =
        match files::collect_files_category(client, repo, Some(branch), Some(&desired)).await {
            Ok(collection) => collection,
            Err(error) => return failure(audit, repo, category, &error),
        };
    let branch_plan = match files::plan_files_category(&desired, &branch_collection) {
        Ok(plan) => plan,
        Err(error) => return failure(audit, repo, category, &error),
    };

    let message = format!("{commit_prefix}sync managed files");

    if !branch_plan.atomic_entries.is_empty()
        && let Err(error) =
            files::apply_files_plan(client, repo, branch, &message, &branch_plan).await
    {
        return failure(audit, repo, category, &error);
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
        Err(error) => return failure(audit, repo, category, &error),
    };

    let mut details = category.details.clone();
    details.push(format!("pull request: {}", pr.html_url));

    // Verify files against the PR branch, never the default branch.
    let verified = match files::verify_files_category(client, repo, Some(branch), &desired).await {
        Ok(result) => result.matches,
        Err(error) => {
            let mut report = failure(audit, repo, category, &error);
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

pub(super) fn files_apply_report(
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
    desired_categories: &ManifestCategories,
    repo: &str,
    category: &CategoryPlan,
    plan: &security_rules::SecurityPlan,
    verify: bool,
    audit: &AuditLog,
) -> CategoryReport {
    if let Err(error) = security_rules::apply_security_plan(client, repo, plan).await {
        return failure(audit, repo, category, &error);
    }
    let verified = if !verify {
        None
    } else if let Some(desired) = desired_categories.security.as_ref() {
        match security_rules::verify_security_category(client, repo, desired).await {
            Ok(result) => Some(result.matches),
            Err(error) => return failure(audit, repo, category, &error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

#[allow(clippy::too_many_arguments)]
async fn apply_actions(
    client: &Client,
    desired_categories: &ManifestCategories,
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
        Err(error) => return failure(audit, repo, category, &error),
    }

    let verified = if let Some(desired) = desired_categories.actions.as_ref() {
        if config_pr_pending {
            match verify_actions_safe_subset(client, repo, desired).await {
                Ok(matches) => Some(matches),
                Err(error) => return failure(audit, repo, category, &error),
            }
        } else {
            match actions_environments::verify_actions_category(client, repo, desired).await {
                Ok(result) => Some(result.compliant),
                Err(error) => return failure(audit, repo, category, &error),
            }
        }
    } else {
        None
    };

    finish_success_deferred(audit, repo, category, verified, deferred, "success")
}

async fn apply_environments(
    client: &Client,
    desired_categories: &ManifestCategories,
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
        Err(error) => return failure(audit, repo, category, &error),
    }
    let verified = if let Some(desired) = desired_categories.environments.as_ref() {
        match actions_environments::verify_environments_category(client, repo, desired).await {
            Ok(result) => Some(result.compliant),
            Err(error) => return failure(audit, repo, category, &error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

async fn apply_access(
    client: &Client,
    desired_categories: &ManifestCategories,
    repo: &str,
    category: &CategoryPlan,
    plan: &access_integrations::AccessPlan,
    audit: &AuditLog,
) -> CategoryReport {
    let report = match access_integrations::apply_access(client, repo, plan).await {
        Ok(report) => report,
        Err(error) => return failure(audit, repo, category, &error),
    };
    if !report.blocked.is_empty() {
        return blocked_with_message(audit, repo, category, report.blocked.join("; "));
    }
    let verified = if let Some(desired) = desired_categories.access.as_ref() {
        match access_integrations::verify_access(client, repo, desired).await {
            Ok(result) => Some(result.is_ok()),
            Err(error) => return failure(audit, repo, category, &error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

#[allow(clippy::too_many_arguments)]
async fn apply_integrations(
    client: &Client,
    desired_categories: &ManifestCategories,
    repo: &str,
    category: &CategoryPlan,
    plan: &access_integrations::IntegrationsPlan,
    config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    let (safe_plan, deferred) = integrations_plan_for_apply(plan, config_pr_pending);

    let report = match access_integrations::apply_integrations(client, repo, &safe_plan).await {
        Ok(report) => report,
        Err(error) => return failure(audit, repo, category, &error),
    };
    if !report.blocked.is_empty() {
        return blocked_with_message(audit, repo, category, report.blocked.join("; "));
    }
    let verified = if let Some(desired) = desired_categories.integrations.as_ref() {
        if config_pr_pending {
            match verify_integrations_safe_subset(client, repo, desired).await {
                Ok(matches) => Some(matches),
                Err(error) => return failure(audit, repo, category, &error),
            }
        } else {
            match access_integrations::verify_integrations(client, repo, desired).await {
                Ok(result) => Some(result.is_ok()),
                Err(error) => return failure(audit, repo, category, &error),
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
    desired_categories: &ManifestCategories,
    repo: &str,
    category: &CategoryPlan,
    plan: &security_rules::RulesetsPlan,
    _config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    if let Err(error) = security_rules::apply_rulesets_plan(client, repo, plan).await {
        return failure(audit, repo, category, &error);
    }
    let verified = if let Some(desired) = desired_categories.rulesets.as_ref() {
        match security_rules::verify_rulesets_category(client, repo, desired).await {
            Ok(result) => Some(result.matches),
            Err(error) => return failure(audit, repo, category, &error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}

#[allow(clippy::too_many_arguments)]
async fn apply_branch_protection(
    client: &Client,
    desired_categories: &ManifestCategories,
    repo: &str,
    category: &CategoryPlan,
    plan: &security_rules::BranchProtectionPlan,
    _config_pr_pending: bool,
    audit: &AuditLog,
) -> CategoryReport {
    if let Err(error) = security_rules::apply_branch_protection_plan(client, repo, plan).await {
        return failure(audit, repo, category, &error);
    }
    let verified = if let Some(desired) = desired_categories.branch_protection.as_ref() {
        match security_rules::verify_branch_protection_category(client, repo, desired).await {
            Ok(result) => Some(result.matches),
            Err(error) => return failure(audit, repo, category, &error),
        }
    } else {
        None
    };
    finish_success(audit, repo, category, verified)
}
