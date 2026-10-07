//! Branch protection reconciliation.

use std::collections::HashMap;

use crate::config::manifest::{BranchProtectionCategory, CoverageEntry, ProtectedBranchConfig};
use crate::github::branch_protection::{
    DesiredBranchProtection, DetailedBranchProtection, StatusCheckRequirement,
};
use crate::reconcile::common::rules_issue::ReconcileIssue;

mod apply;
mod collect;
mod plan;

pub use apply::*;
pub use collect::*;
pub use plan::*;

#[derive(Debug, Clone, PartialEq)]
pub struct ActualProtectedBranch {
    pub name: String,
    pub is_default_branch: bool,
    pub manifest: ProtectedBranchConfig,
    pub raw: DetailedBranchProtection,
    pub status_checks: Vec<StatusCheckRequirement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BranchProtectionCollection {
    pub default_branch_name: String,
    pub category: BranchProtectionCategory,
    pub actual_branches: Vec<ActualProtectedBranch>,
    pub app_ids_by_slug: HashMap<String, i64>,
    pub app_slugs_by_id: HashMap<i64, String>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchProtectionPlanAction {
    Upsert {
        branch: String,
        desired: Box<DesiredBranchProtection>,
    },
    Delete {
        branch: String,
    },
    Unchanged {
        branch: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchProtectionPlan {
    pub actions: Vec<BranchProtectionPlanAction>,
    pub issues: Vec<ReconcileIssue>,
}

impl BranchProtectionPlan {
    pub fn has_changes(&self) -> bool {
        self.actions
            .iter()
            .any(|action| !matches!(action, BranchProtectionPlanAction::Unchanged { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchProtectionApplyResult {
    pub applied_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchProtectionVerifyResult {
    pub matches: bool,
    pub plan: BranchProtectionPlan,
}
