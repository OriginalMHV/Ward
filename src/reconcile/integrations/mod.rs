//! Integrations snapshot and reconciliation.

use crate::config::manifest::{
    AutolinkConfig, CategoryPolicy, CoverageEntry, DeployKeyConfig, PagesConfig,
    RepositoryIntegrationsCategory, WebhookConfig,
};
use crate::reconcile::common::issue::ReconcileIssue;

mod apply;
mod collect;
mod normalize;
mod plan;

pub use apply::{apply_integrations, verify_integrations, verify_integrations_state};
pub use collect::collect_integrations;
pub use normalize::canonicalize_url;
pub use plan::plan_integrations;

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationsCollection {
    pub category: RepositoryIntegrationsCategory,
    pub state: CollectedIntegrationsState,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedIntegrationsState {
    pub webhooks: Vec<CollectedWebhook>,
    pub webhooks_complete: bool,
    pub deploy_keys: Vec<CollectedDeployKey>,
    pub deploy_keys_complete: bool,
    pub pages: Option<CollectedPages>,
    pub pages_complete: bool,
    pub autolinks: Vec<CollectedAutolink>,
    pub autolinks_complete: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedWebhook {
    pub id: u64,
    pub config: WebhookConfig,
    pub canonical_url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedDeployKey {
    pub id: u64,
    pub config: DeployKeyConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedPages {
    pub config: PagesConfig,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedAutolink {
    pub id: u64,
    pub config: AutolinkConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationsPlan {
    pub policy: CategoryPolicy,
    pub webhook_actions: Vec<WebhookAction>,
    pub deploy_key_actions: Vec<DeployKeyAction>,
    pub pages_action: Option<PagesAction>,
    pub autolink_actions: Vec<AutolinkAction>,
    pub notes: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

impl IntegrationsPlan {
    pub fn is_empty(&self) -> bool {
        self.webhook_actions.is_empty()
            && self.deploy_key_actions.is_empty()
            && self.pages_action.is_none()
            && self.autolink_actions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebhookAction {
    Create(WebhookConfig),
    Update {
        hook_id: u64,
        current: CollectedWebhook,
        desired: WebhookConfig,
    },
    Delete {
        hook_id: u64,
        redacted_url: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeployKeyAction {
    Create(DeployKeyConfig),
    Replace {
        key_id: u64,
        current_title: String,
        desired: DeployKeyConfig,
    },
    Delete {
        key_id: u64,
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PagesAction {
    Create(PagesConfig),
    Update(PagesConfig),
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AutolinkAction {
    Create(AutolinkConfig),
    Recreate {
        autolink_id: u64,
        desired: AutolinkConfig,
    },
    Delete {
        autolink_id: u64,
        key_prefix: String,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IntegrationsApplyReport {
    pub applied: Vec<String>,
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IntegrationsVerification {
    pub issues: Vec<String>,
    pub notes: Vec<String>,
}

impl IntegrationsVerification {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}
