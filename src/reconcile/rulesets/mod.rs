//! Repository ruleset reconciliation.

use std::collections::HashMap;

use crate::config::manifest::{CoverageEntry, RepositoryRuleset, RulesetsCategory};
use crate::reconcile::common::rules_issue::ReconcileIssue;

mod apply;
mod collect;
mod plan;

pub use apply::*;
pub use collect::*;
pub use plan::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetReference {
    pub name: String,
    pub target: String,
    pub enforcement: String,
    pub source_type: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActualRepositoryRuleset {
    pub id: u64,
    pub source_type: String,
    pub source: String,
    pub ruleset: RepositoryRuleset,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RulesetsCollection {
    pub category: RulesetsCategory,
    pub actual_repository_rulesets: Vec<ActualRepositoryRuleset>,
    pub inherited_rulesets: Vec<RulesetReference>,
    pub team_ids_by_slug: HashMap<String, u64>,
    pub repository_role_ids_by_name: HashMap<String, u64>,
    pub app_ids_by_slug: HashMap<String, u64>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RulesetPlanAction {
    Create {
        ruleset: RepositoryRuleset,
    },
    Update {
        ruleset_id: u64,
        ruleset: RepositoryRuleset,
    },
    Delete {
        ruleset_id: u64,
        name: String,
    },
    Unchanged {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RulesetsPlan {
    pub actions: Vec<RulesetPlanAction>,
    pub issues: Vec<ReconcileIssue>,
}

impl RulesetsPlan {
    pub fn has_changes(&self) -> bool {
        self.actions
            .iter()
            .any(|action| !matches!(action, RulesetPlanAction::Unchanged { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetsApplyResult {
    pub applied_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RulesetsVerifyResult {
    pub matches: bool,
    pub plan: RulesetsPlan,
}
