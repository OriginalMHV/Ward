//! Environments category: types and phase modules.

mod apply;
mod collect;
mod plan;

pub use apply::{apply_environments_plan, verify_environments_category};
pub use collect::collect_environments_category;
pub use plan::{plan_environments_category, plan_environments_category_with_env};

use super::common::issue::ReconcileIssue;
use super::common::secrets::ResolvedSecret;
use crate::config::manifest::{
    ActorReference, CoverageEntry, EnvironmentsCategory, NamedValueConfig,
};
use crate::github::environments::DeploymentBranchPolicySummary;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct EnvironmentsCollection {
    pub category: EnvironmentsCategory,
    /// Every environment name observed on the repository, regardless of
    /// whether it was in `desired` and therefore deep-collected. Needed so
    /// `plan_environments_category` can detect prune candidates that were
    /// filtered out of `category.entries` for collection efficiency.
    pub observed_names: Vec<String>,
    /// Numeric GitHub ids for observed deployment branch/tag policy patterns,
    /// keyed by `(environment, policy_type, pattern_name)` where `policy_type`
    /// is `"branch"` or `"tag"`. Retained only in collection state so prune
    /// can delete a pattern by id while the manifest itself stays free of
    /// volatile GitHub ids.
    pub deployment_policy_ids: BTreeMap<(String, String, String), u64>,
    /// Environments whose deployment-branch-policy listing was fully read
    /// (no 403/404/422 coverage gap). Only these are eligible for pattern
    /// prune; an incomplete read must never trigger deletions.
    pub deployment_policies_observed: std::collections::BTreeSet<String>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvironmentSettingsChange {
    pub wait_timer_minutes: Option<u32>,
    pub prevent_self_review: Option<bool>,
    pub reviewers: Vec<ActorReference>,
    pub deployment_branch_policy: Option<DeploymentBranchPolicySummary>,
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentPlan {
    pub name: String,
    pub create: bool,
    pub settings_change: Option<EnvironmentSettingsChange>,
    pub branch_policy_creates: Vec<(String, String)>,
    pub branch_policy_deletes: Vec<u64>,
    pub protection_app_enables: Vec<String>,
    /// App slugs to disable (resolved to a numeric protection-rule id at
    /// apply time, since the collected state only carries slugs).
    pub protection_app_disables: Vec<String>,
    pub variable_upserts: Vec<NamedValueConfig>,
    pub variable_deletions: Vec<String>,
    pub secret_upserts: Vec<ResolvedSecret>,
    pub secret_deletions: Vec<String>,
}

impl EnvironmentPlan {
    pub fn has_actionable_changes(&self) -> bool {
        self.create
            || self.settings_change.is_some()
            || !self.branch_policy_creates.is_empty()
            || !self.branch_policy_deletes.is_empty()
            || !self.protection_app_enables.is_empty()
            || !self.protection_app_disables.is_empty()
            || !self.variable_upserts.is_empty()
            || !self.variable_deletions.is_empty()
            || !self.secret_upserts.is_empty()
            || !self.secret_deletions.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentsPlan {
    pub environment_plans: Vec<EnvironmentPlan>,
    pub environment_deletions: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

impl EnvironmentsPlan {
    pub fn has_actionable_changes(&self) -> bool {
        !self.environment_deletions.is_empty()
            || self
                .environment_plans
                .iter()
                .any(EnvironmentPlan::has_actionable_changes)
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentsApplyResult {
    pub applied: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone)]
pub struct EnvironmentsVerifyResult {
    pub compliant: bool,
    pub plan: EnvironmentsPlan,
}
