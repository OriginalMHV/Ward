//! Security category reconciliation.

use std::collections::HashMap;

use crate::config::manifest::{CoverageEntry, SecurityCategory};
use crate::github::security::{
    CodeSecurityConfiguration, CodeqlDefaultSetupState, RepositoryCodeSecurityConfiguration,
    SecurityAndAnalysisState,
};
use crate::reconcile::common::rules_issue::ReconcileIssue;

mod apply;
mod collect;
mod plan;

pub use apply::*;
pub use collect::*;
pub use plan::*;

#[derive(Debug, Clone, PartialEq)]
pub struct SecurityCollection {
    pub repository_id: u64,
    pub category: SecurityCategory,
    pub analysis: SecurityAndAnalysisState,
    pub private_vulnerability_reporting: Option<bool>,
    pub codeql_default_setup: Option<CodeqlDefaultSetupState>,
    pub attached_configuration: Option<RepositoryCodeSecurityConfiguration>,
    pub available_configurations: Vec<CodeSecurityConfiguration>,
    pub team_ids_by_slug: HashMap<String, u64>,
    pub repository_role_ids_by_name: HashMap<String, u64>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityPlan {
    pub repository_id: u64,
    pub patch_security_and_analysis: Option<serde_json::Value>,
    pub dependabot_alerts: Option<bool>,
    pub dependabot_security_updates: Option<bool>,
    pub private_vulnerability_reporting: Option<bool>,
    pub codeql_default_setup: Option<CodeqlDefaultSetupState>,
    pub attach_configuration_id: Option<u64>,
    pub detach_configuration: bool,
    pub issues: Vec<ReconcileIssue>,
}

impl SecurityPlan {
    pub fn has_changes(&self) -> bool {
        self.patch_security_and_analysis.is_some()
            || self.dependabot_alerts.is_some()
            || self.dependabot_security_updates.is_some()
            || self.private_vulnerability_reporting.is_some()
            || self.codeql_default_setup.is_some()
            || self.attach_configuration_id.is_some()
            || self.detach_configuration
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityApplyResult {
    pub applied_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityVerifyResult {
    pub matches: bool,
    pub plan: SecurityPlan,
}
