use anyhow::Result;

use crate::config::Manifest;
use crate::config::manifest::{
    ActionsCategory, ManagementDisposition, RepositoryIntegrationsCategory,
};
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::reconcile::{access_integrations, actions_environments};

use super::model::{CategoryPlan, CategoryPlanKind, DependencyDeferral};
use super::report::CategoryReport;

// ---------------------------------------------------------------------------
// Apply-side helpers
// ---------------------------------------------------------------------------

pub(super) fn finish_success(
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

pub(super) fn finish_success_deferred(
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
pub(super) fn actions_safe_subset(
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

pub(super) fn actions_plan_for_apply(
    plan: &actions_environments::ActionsPlan,
    config_pr_pending: bool,
) -> (actions_environments::ActionsPlan, usize) {
    if config_pr_pending {
        actions_safe_subset(plan)
    } else {
        (plan.clone(), 0)
    }
}

pub(super) async fn verify_actions_safe_subset(
    client: &Client,
    repo: &str,
    desired: &ActionsCategory,
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
pub(super) fn integrations_safe_subset(
    plan: &access_integrations::IntegrationsPlan,
) -> (access_integrations::IntegrationsPlan, usize) {
    let deferred = usize::from(plan.pages_action.is_some());
    let mut safe = plan.clone();
    safe.pages_action = None;
    (safe, deferred)
}

pub(super) fn integrations_plan_for_apply(
    plan: &access_integrations::IntegrationsPlan,
    config_pr_pending: bool,
) -> (access_integrations::IntegrationsPlan, usize) {
    if config_pr_pending {
        integrations_safe_subset(plan)
    } else {
        (plan.clone(), 0)
    }
}

pub(super) async fn verify_integrations_safe_subset(
    client: &Client,
    repo: &str,
    desired: &RepositoryIntegrationsCategory,
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

pub(super) fn dependency_deferral(
    category: &CategoryPlan,
    config_pr_pending: bool,
) -> DependencyDeferral {
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

pub(super) fn adjust_for_config_pr(
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

pub(super) fn blocked_with_message(
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

pub(super) fn failure(
    audit: &AuditLog,
    repo: &str,
    category: &CategoryPlan,
    error: &anyhow::Error,
) -> CategoryReport {
    let message = format!("{error:#}");
    audit_category(audit, repo, category, "failed", 0, Some(message.clone()));
    let mut report = category.to_report("failed", 0, None);
    report.error = Some(message);
    report
}

pub(super) fn audit_category(
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

pub(super) fn sync_branch(manifest: &Manifest) -> String {
    let branch = manifest.file_delivery.branch.trim();
    if branch.is_empty() {
        "chore/ward-sync".to_owned()
    } else {
        branch.to_owned()
    }
}

pub(super) fn commit_prefix(manifest: &Manifest) -> String {
    let prefix = manifest.file_delivery.commit_message_prefix.trim_end();
    if prefix.is_empty() {
        "chore: ".to_owned()
    } else {
        format!("{prefix} ")
    }
}
