use crate::config::manifest::{CoverageEntry, CoverageOutcome, ManagementDisposition};
use crate::reconcile::{access_integrations, actions_environments, files, general, security_rules};

use super::Category;
use super::gating::{adjust_for_config_pr, dependency_deferral};
use super::report::{CategoryReport, CoverageCounts, CoverageOutcomeCount, RepoReport};

// ---------------------------------------------------------------------------
// Internal typed plan (retained so apply can execute without re-planning)
// ---------------------------------------------------------------------------

pub(super) enum CategoryPlanKind {
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

pub(super) struct CategoryPlan {
    pub(super) name: Category,
    pub(super) disposition: ManagementDisposition,
    pub(super) is_blocked_collection: bool,
    pub(super) coverage: Vec<CoverageEntry>,
    pub(super) actionable: usize,
    pub(super) blocked: usize,
    pub(super) warnings: usize,
    pub(super) details: Vec<String>,
    pub(super) kind: CategoryPlanKind,
}

impl CategoryPlan {
    pub(super) fn disposition_label(&self) -> String {
        if self.is_blocked_collection {
            return "blocked".to_owned();
        }
        disposition_label(self.disposition)
    }

    pub(super) fn plan_status(&self) -> &'static str {
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

    pub(super) fn to_report(
        &self,
        status: &str,
        deferred: usize,
        verified: Option<bool>,
    ) -> CategoryReport {
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

pub(super) struct RepoPlan {
    pub(super) repo: String,
    pub(super) default_branch: String,
    pub(super) categories: Vec<CategoryPlan>,
}

impl RepoPlan {
    #[cfg(test)]
    pub(super) fn to_report(&self) -> RepoReport {
        self.to_report_with_config_pr(false)
    }

    pub(super) fn to_report_with_config_pr(&self, existing_config_pr: bool) -> RepoReport {
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

    pub(super) fn plans_config_pull_request(&self) -> bool {
        self.categories.iter().any(|category| {
            category.name == Category::Files
                && category.disposition == ManagementDisposition::Managed
                && category.actionable > 0
                && category.blocked == 0
        })
    }

    /// An unreadable PR list degrades the file-dependent categories to
    /// unavailable coverage instead of aborting the whole plan.
    pub(super) fn record_config_pr_lookup_failure(&mut self, message: &str) {
        for category in &mut self.categories {
            if dependency_deferral(category, true).total == 0 {
                continue;
            }
            let name = match category.name {
                Category::Actions => crate::config::manifest::ManifestCategoryName::Actions,
                Category::Integrations => {
                    crate::config::manifest::ManifestCategoryName::Integrations
                }
                Category::Rulesets => crate::config::manifest::ManifestCategoryName::Rulesets,
                _ => crate::config::manifest::ManifestCategoryName::BranchProtection,
            };
            category.coverage.push(CoverageEntry {
                category: name,
                endpoint: "pulls (open configuration pull request)".to_owned(),
                outcome: CoverageOutcome::Unavailable,
                reason: Some(message.to_owned()),
                required_permission: Some("pull_requests:read".to_owned()),
            });
            category.warnings += 1;
        }
    }

    pub(super) fn has_file_dependent_changes(&self) -> bool {
        self.categories
            .iter()
            .any(|category| dependency_deferral(category, true).total > 0)
    }
}

#[derive(Debug, Default)]
pub(super) struct DependencyDeferral {
    pub(super) actionable: usize,
    pub(super) blockers: usize,
    pub(super) total: usize,
}

pub(super) fn aggregate_repo(repo: String, categories: Vec<CategoryReport>) -> RepoReport {
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

pub(super) const DETAIL_LIMIT: usize = 8;

pub(super) fn disposition_label(disposition: ManagementDisposition) -> String {
    match disposition {
        ManagementDisposition::Managed => "managed",
        ManagementDisposition::Reference => "reference",
        ManagementDisposition::Placeholder => "placeholder",
        ManagementDisposition::Observe => "observe",
    }
    .to_owned()
}

pub(super) fn degraded_coverage(coverage: &[CoverageEntry]) -> usize {
    coverage
        .iter()
        .filter(|entry| {
            matches!(
                entry.outcome,
                CoverageOutcome::PermissionDenied | CoverageOutcome::Unavailable
            )
        })
        .count()
}

pub(super) fn coverage_counts(coverage: &[CoverageEntry]) -> CoverageCounts {
    let mut counts = CoverageCounts {
        total: coverage.len(),
        ..CoverageCounts::default()
    };
    for entry in coverage {
        match entry.outcome {
            CoverageOutcome::Collected => counts.collected += 1,
            CoverageOutcome::NotApplicable => counts.not_applicable += 1,
            CoverageOutcome::PermissionDenied | CoverageOutcome::Unavailable => {
                counts.degraded += 1;
            }
            CoverageOutcome::Redacted | CoverageOutcome::Unsupported => counts.unsupported += 1,
        }
    }
    counts
}

pub(super) fn coverage_outcomes(coverage: &[CoverageEntry]) -> Vec<CoverageOutcomeCount> {
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

pub(super) fn reconcile_issue_counts(issues: &[security_rules::ReconcileIssue]) -> (usize, usize) {
    let blocked = issues
        .iter()
        .filter(|issue| issue.severity == security_rules::ReconcileIssueSeverity::Blocker)
        .count();
    let warnings = issues.len() - blocked;
    (blocked, warnings)
}

pub(super) fn actions_issue_counts(
    issues: &[actions_environments::ReconcileIssue],
) -> (usize, usize) {
    let blocked = issues
        .iter()
        .filter(|issue| issue.severity == actions_environments::IssueSeverity::Blocker)
        .count();
    let warnings = issues.len() - blocked;
    (blocked, warnings)
}

pub(super) fn actions_actionable_count(plan: &actions_environments::ActionsPlan) -> usize {
    plan.settings_changes.len()
        + plan.workflow_state_changes.len()
        + plan.variable_upserts.len()
        + plan.variable_deletions.len()
        + plan.secret_upserts.len()
        + plan.secret_deletions.len()
        + plan.reference_actions.len()
}

pub(super) fn environments_actionable_count(
    plan: &actions_environments::EnvironmentsPlan,
) -> usize {
    plan.environment_deletions.len()
        + plan
            .environment_plans
            .iter()
            .filter(|env| env.has_actionable_changes())
            .count()
}

pub(super) fn describe_general_change(change: &general::GeneralChange) -> String {
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

pub(super) fn security_change_details(plan: &security_rules::SecurityPlan) -> Vec<String> {
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

pub(super) fn ruleset_action_details(plan: &security_rules::RulesetsPlan) -> Vec<String> {
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

pub(super) fn branch_protection_details(
    plan: &security_rules::BranchProtectionPlan,
) -> Vec<String> {
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

pub(super) fn actions_details(plan: &actions_environments::ActionsPlan) -> Vec<String> {
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
