//! Actions category: types and phase modules.

mod apply;
mod collect;
mod plan;
mod references;

pub use apply::{apply_actions_plan, verify_actions_category};
pub use collect::collect_actions_category;
pub use plan::{plan_actions_category, plan_actions_category_with_env};

use super::common::issue::ReconcileIssue;
use super::common::secrets::ResolvedSecret;
use crate::config::manifest::{
    ActionsCategory, CoverageEntry, NamedValueConfig, ReferencedResourceConfig,
};

#[derive(Debug, Clone, Default)]
pub struct ActionsCollection {
    pub category: ActionsCategory,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
    /// Resolution of organization secret/variable *references* against the
    /// target organization (existence by stable name, and — for `selected`
    /// visibility — whether this repository is associated). Populated for
    /// every name in `desired.references` plus, when `desired.policy.prune`
    /// is set, every organization secret/variable currently visible to this
    /// repository. See [`ResolvedOrgReference`].
    pub resolved_references: Vec<ResolvedOrgReference>,
}

/// The resolved state of a referenced organization secret or variable
/// against the target organization, produced during collection (network
/// access happens here, never in `plan_actions_category`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOrgReference {
    pub resource: ReferencedResourceConfig,
    /// `None` = the existence lookup was unavailable (permission denied or
    /// otherwise) and must never be treated as "absent". `Some(false)` = no
    /// resource by this stable name exists in the target organization.
    /// `Some(true)` = it exists.
    pub present: Option<bool>,
    /// Whether this repository is associated. Always `None` when
    /// `supported` is `false` (visibility is `all`/`private`, so there is
    /// nothing to associate/disassociate — the repository already has
    /// access by virtue of visibility, which ward never alters). When
    /// `supported` is `true` (visibility is `selected`), `None` means the
    /// selected-repositories list could not be resolved and association
    /// state is genuinely unknown — never assumed either way.
    pub associated: Option<bool>,
    /// `true` when visibility is `selected`, i.e. per-repository
    /// association is an actionable GitHub API concept for this resource.
    pub supported: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionsSettingChange {
    Permissions {
        enabled: bool,
        allowed_actions: Option<String>,
        sha_pinning_required: Option<bool>,
    },
    SelectedActions {
        github_owned_allowed: bool,
        verified_allowed: bool,
        patterns_allowed: Vec<String>,
    },
    WorkflowPermissions {
        default_workflow_permissions: String,
        can_approve_pull_request_reviews: bool,
    },
    ArtifactLogRetention {
        days: u32,
    },
    CacheRetentionLimit {
        max_cache_retention_days: u32,
    },
    CacheStorageLimit {
        max_cache_size_gb: u32,
    },
    ForkPrContributorApproval {
        approval_policy: String,
    },
    PrivateForkPrWorkflows {
        run_workflows_from_fork_pull_requests: bool,
        // `None` means the manifest doesn't specify this field: apply must
        // preserve whatever value is live rather than resetting it to
        // `false`. `Some(x)` is an explicit desired override.
        send_write_tokens_to_workflows: Option<bool>,
        send_secrets_and_variables: Option<bool>,
        require_approval_for_fork_pr_workflows: Option<bool>,
    },
    WorkflowAccessLevel {
        access_level: String,
    },
    OidcSubjectClaim {
        use_default: bool,
        include_claim_keys: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStateChange {
    pub path: String,
    pub enabled: bool,
}

/// An association change for a referenced organization secret/variable.
/// Ward never issues any write against the org resource's own value or
/// visibility here — only the per-repository `selected` association is
/// ever touched, and only under the policy gates enforced in
/// `plan_actions_category`/`apply_actions_plan` (`Associate` requires
/// `managed` + `sensitive`; `Disassociate` additionally requires `prune`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgReferenceAction {
    Associate(ReferencedResourceConfig),
    Disassociate(ReferencedResourceConfig),
}

#[derive(Debug, Clone)]
pub struct ActionsPlan {
    pub settings_changes: Vec<ActionsSettingChange>,
    pub workflow_state_changes: Vec<WorkflowStateChange>,
    pub variable_upserts: Vec<NamedValueConfig>,
    pub variable_deletions: Vec<String>,
    pub secret_upserts: Vec<ResolvedSecret>,
    pub secret_deletions: Vec<String>,
    pub reference_actions: Vec<OrgReferenceAction>,
    pub issues: Vec<ReconcileIssue>,
}

impl ActionsPlan {
    pub fn has_actionable_changes(&self) -> bool {
        !self.settings_changes.is_empty()
            || !self.workflow_state_changes.is_empty()
            || !self.variable_upserts.is_empty()
            || !self.variable_deletions.is_empty()
            || !self.secret_upserts.is_empty()
            || !self.secret_deletions.is_empty()
            || !self.reference_actions.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActionsApplyResult {
    pub applied: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone)]
pub struct ActionsVerifyResult {
    pub compliant: bool,
    pub plan: ActionsPlan,
}
