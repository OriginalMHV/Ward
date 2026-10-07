//! Access snapshot and reconciliation.

use crate::config::manifest::{
    CategoryPolicy, CollaboratorAccessConfig, CoverageEntry, ReferencedResourceConfig,
    RepositoryAccessCategory, TeamAccess,
};
use crate::reconcile::common::issue::ReconcileIssue;

mod apply;
mod collect;
mod plan;

pub use apply::{apply_access, verify_access, verify_access_state};
pub use collect::collect_access;
pub use plan::plan_access;

#[derive(Debug, Clone, PartialEq)]
pub struct AccessCollection {
    pub category: RepositoryAccessCategory,
    pub state: CollectedAccessState,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedAccessState {
    pub teams: Vec<TeamAccess>,
    pub teams_complete: bool,
    pub collaborators: Vec<CollectedCollaborator>,
    pub collaborators_complete: bool,
    pub references: Vec<CollectedAccessReference>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedCollaborator {
    pub config: CollaboratorAccessConfig,
    pub outside: bool,
    pub pending: bool,
    pub invitation_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedAccessReference {
    pub resource: ReferencedResourceConfig,
    pub present: Option<bool>,
    pub associated: Option<bool>,
    pub supported: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessPlan {
    pub policy: CategoryPolicy,
    pub team_actions: Vec<TeamAccessAction>,
    pub collaborator_actions: Vec<CollaboratorAccessAction>,
    pub reference_actions: Vec<AccessReferenceAction>,
    pub notes: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TeamAccessAction {
    Ensure(TeamAccess),
    Remove(TeamAccess),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CollaboratorAccessAction {
    Grant(CollaboratorAccessConfig),
    Reinvite {
        invitation_id: u64,
        desired: CollaboratorAccessConfig,
    },
    Revoke {
        login: String,
        invitation_id: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccessReferenceAction {
    Associate(ReferencedResourceConfig),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccessApplyReport {
    pub applied: Vec<String>,
    pub pending: Vec<String>,
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccessVerification {
    pub issues: Vec<String>,
    pub pending: Vec<String>,
    pub notes: Vec<String>,
}

impl AccessVerification {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}
