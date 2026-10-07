//! General repository settings snapshot/reconciliation.

mod apply;
mod collect;
mod plan;
mod resources;

pub use apply::*;
pub use collect::*;
pub use plan::*;

use crate::config::manifest::{CoverageEntry, LabelConfig, RepositoryCategory};
use crate::github::settings::GraphqlRepositoryPatch;
use resources::extract_manifest_custom_properties;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GeneralDesiredExtensions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_pull_requests: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request_creation_policy: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_sponsorships_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_creation_policy: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_squash_pr_title_as_default: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GeneralCustomPropertyValue {
    pub property_name: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GeneralLabel {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default)]
    pub default: bool,
}

impl From<LabelConfig> for GeneralLabel {
    fn from(value: LabelConfig) -> Self {
        Self {
            name: value.name,
            color: value.color,
            description: value.description,
            default: value.default.unwrap_or(false),
        }
    }
}

impl From<GeneralLabel> for LabelConfig {
    fn from(value: GeneralLabel) -> Self {
        Self {
            name: value.name,
            color: value.color,
            description: value.description,
            default: Some(value.default),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GeneralDesiredState {
    pub repository: RepositoryCategory,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<GeneralLabel>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<GeneralCustomPropertyValue>,

    #[serde(default)]
    pub extensions: GeneralDesiredExtensions,
}

impl From<RepositoryCategory> for GeneralDesiredState {
    fn from(repository: RepositoryCategory) -> Self {
        let custom_properties = extract_manifest_custom_properties(&repository);
        Self {
            repository,
            labels: Vec::new(),
            custom_properties,
            extensions: GeneralDesiredExtensions::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GeneralCollectedExtensions {
    #[serde(default)]
    pub repository_id: String,

    #[serde(default)]
    pub graphql_settings_collected: bool,

    #[serde(default)]
    pub labels_collected: bool,

    #[serde(default)]
    pub custom_properties_collected: bool,

    #[serde(default)]
    pub immutable_releases_collected: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_discussions_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_pull_requests: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request_creation_policy: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_sponsorships_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_creation_policy: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_squash_pr_title_as_default: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CollectedGeneralState {
    pub repository: RepositoryCategory,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<GeneralLabel>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<GeneralCustomPropertyValue>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageEntry>,

    #[serde(default)]
    pub extensions: GeneralCollectedExtensions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GeneralPlanOptions {
    #[serde(default)]
    pub allow_high_impact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneralChangeKind {
    RestField {
        field: String,
    },
    GraphqlField {
        field: String,
    },
    Topics,
    CustomProperty {
        property_name: String,
        action: GeneralResourceAction,
    },
    ImmutableReleases {
        action: GeneralResourceAction,
    },
    Label {
        name: String,
        action: GeneralResourceAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralResourceAction {
    Create,
    Update,
    Delete,
    Enable,
    Disable,
    Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GeneralChange {
    pub kind: GeneralChangeKind,
    pub current: String,
    pub desired: String,
    #[serde(default)]
    pub high_impact: bool,
    #[serde(default)]
    pub reference_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PlannedCustomPropertyUpdate {
    pub property_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PlannedImmutableReleaseAction {
    Enable,
    Disable,
    Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PlannedLabelAction {
    Create {
        label: GeneralLabel,
    },
    Update {
        current_name: String,
        label: GeneralLabel,
    },
    Delete {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GeneralPlan {
    pub repo: String,
    pub repository_id: String,
    pub desired: GeneralDesiredState,
    #[serde(default)]
    pub options: GeneralPlanOptions,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageEntry>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<GeneralChange>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_changes: Vec<GeneralChange>,

    pub rest_patch: Value,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphql_patch: Option<GraphqlRepositoryPatch>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_property_updates: Vec<PlannedCustomPropertyUpdate>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub immutable_releases: Option<PlannedImmutableReleaseAction>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_actions: Vec<PlannedLabelAction>,
}

impl GeneralPlan {
    pub fn has_actionable_changes(&self) -> bool {
        self.rest_patch
            .as_object()
            .is_some_and(|body| !body.is_empty())
            || self
                .graphql_patch
                .as_ref()
                .is_some_and(|patch| !patch.is_empty())
            || self.topics.is_some()
            || !self.custom_property_updates.is_empty()
            || self.immutable_releases.as_ref().is_some_and(|action| {
                matches!(
                    action,
                    PlannedImmutableReleaseAction::Enable | PlannedImmutableReleaseAction::Disable
                )
            })
            || !self.label_actions.is_empty()
    }

    pub fn has_blocked_changes(&self) -> bool {
        !self.blocked_changes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GeneralVerification {
    pub repo: String,
    pub compliant: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<CoverageEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_changes: Vec<GeneralChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_changes: Vec<GeneralChange>,
}
