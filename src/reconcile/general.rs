//! General repository settings snapshot/reconciliation.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::manifest::{
    CategoryPolicy, CoverageEntry, CoverageOutcome, CustomPropertyValueConfig,
    ImmutableReleasesConfig, LabelConfigV2, ManagementDisposition, ManifestCategoryName,
    RepositoryCategoryV2, RepositoryMetadataConfig, RepositorySettingsConfig,
};
use crate::github::Client;
use crate::github::settings::{
    ClassifiedApiResponse, CustomPropertyValueMutation, GraphqlRepositoryPatch,
    GraphqlRepositorySettings, ImmutableReleasesState, RepositoryCustomPropertyValue,
    RepositoryGeneralSettings, RepositoryLabel,
};

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

impl From<LabelConfigV2> for GeneralLabel {
    fn from(value: LabelConfigV2) -> Self {
        Self {
            name: value.name,
            color: value.color,
            description: value.description,
            default: value.default.unwrap_or(false),
        }
    }
}

impl From<GeneralLabel> for LabelConfigV2 {
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
    pub repository: RepositoryCategoryV2,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<GeneralLabel>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<GeneralCustomPropertyValue>,

    #[serde(default)]
    pub extensions: GeneralDesiredExtensions,
}

impl From<RepositoryCategoryV2> for GeneralDesiredState {
    fn from(repository: RepositoryCategoryV2) -> Self {
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
    pub repository: RepositoryCategoryV2,

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

pub async fn collect(client: &Client, repo: &str) -> Result<CollectedGeneralState> {
    let rest = client.get_repository_general_settings(repo).await?;
    let mut coverage = unsupported_repository_settings_coverage();

    let graphql = match client
        .get_repository_graphql_settings_classified(repo)
        .await?
    {
        ClassifiedApiResponse::Success(settings) => Some(settings),
        ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::PermissionDenied,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Unprocessable(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Conflict(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NoContent => None,
    };

    let topics = match client.get_topics_classified(repo).await? {
        ClassifiedApiResponse::Success(values) => Some(normalize_topics(&values)),
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/topics",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("read".to_owned()),
            ));
            None
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/topics",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/topics",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NoContent => Some(Vec::new()),
    };

    let custom_properties = match client.get_custom_property_values(repo).await? {
        ClassifiedApiResponse::Success(values) => collect_custom_properties(&values),
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/properties/values",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("read".to_owned()),
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/properties/values",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/properties/values",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NoContent => Vec::new(),
    };

    let immutable_releases = match client.get_immutable_releases_state_classified(repo).await? {
        ClassifiedApiResponse::Success(state) => Some(state),
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/immutable-releases",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("admin".to_owned()),
            ));
            None
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/immutable-releases",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/immutable-releases",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NoContent => Some(ImmutableReleasesState {
            enabled: false,
            enforced_by_owner: false,
        }),
    };

    let labels = match client.list_labels_classified(repo).await? {
        ClassifiedApiResponse::Success(labels) => collect_labels(labels),
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/labels",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("read".to_owned()),
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/labels",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/labels",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NoContent => Vec::new(),
    };

    let repository = RepositoryCategoryV2 {
        policy: CategoryPolicy::managed(),
        settings: Some(build_repository_settings(
            &rest,
            topics.as_ref(),
            graphql.as_ref(),
        )?),
        metadata: Some(build_repository_metadata(&rest)?),
        custom_properties: manifest_compatible_custom_properties(&custom_properties),
        immutable_releases: immutable_releases
            .as_ref()
            .map(|state| ImmutableReleasesConfig {
                enabled: Some(state.enabled),
                enforced_by_owner: Some(state.enforced_by_owner),
            }),
        references: Vec::new(),
    };

    let labels_collected = !has_coverage_entry(&coverage, "GET /repos/{owner}/{repo}/labels");
    let custom_properties_collected =
        !has_coverage_entry(&coverage, "GET /repos/{owner}/{repo}/properties/values");

    Ok(CollectedGeneralState {
        repository,
        labels,
        custom_properties,
        coverage,
        extensions: GeneralCollectedExtensions {
            repository_id: rest.node_id,
            graphql_settings_collected: graphql.is_some(),
            labels_collected,
            custom_properties_collected,
            immutable_releases_collected: immutable_releases.is_some(),
            has_discussions_enabled: Some(
                graphql
                    .as_ref()
                    .map_or(rest.has_discussions, |value| value.has_discussions_enabled),
            ),
            has_pull_requests: Some(rest.has_pull_requests),
            pull_request_creation_policy: normalize_optional_policy(
                rest.pull_request_creation_policy.as_deref(),
            ),
            has_sponsorships_enabled: graphql.as_ref().map(|value| value.has_sponsorships_enabled),
            issue_creation_policy: graphql.as_ref().and_then(|value| {
                normalize_optional_policy(value.issue_creation_policy.as_deref())
            }),
            use_squash_pr_title_as_default: rest.use_squash_pr_title_as_default,
        },
    })
}

pub fn plan(
    repo: &str,
    desired: &GeneralDesiredState,
    current: &CollectedGeneralState,
) -> GeneralPlan {
    plan_with_options(repo, desired, current, GeneralPlanOptions::default())
}

pub fn plan_with_options(
    repo: &str,
    desired: &GeneralDesiredState,
    current: &CollectedGeneralState,
    options: GeneralPlanOptions,
) -> GeneralPlan {
    if desired.repository.policy.disposition != ManagementDisposition::Managed {
        return GeneralPlan {
            repo: repo.to_owned(),
            repository_id: current.extensions.repository_id.clone(),
            desired: desired.clone(),
            options,
            coverage: current.coverage.clone(),
            changes: Vec::new(),
            blocked_changes: Vec::new(),
            rest_patch: Value::Object(Map::new()),
            graphql_patch: None,
            topics: None,
            custom_property_updates: Vec::new(),
            immutable_releases: None,
            label_actions: Vec::new(),
        };
    }

    let current_settings = current.repository.settings.clone().unwrap_or_default();
    let current_metadata = current.repository.metadata.clone().unwrap_or_default();
    let allow_high_impact = options.allow_high_impact || desired.repository.policy.sensitive;

    let mut rest_patch = Map::new();
    let mut graphql_patch = GraphqlRepositoryPatch::default();
    let mut changes = Vec::new();
    let mut blocked_changes = Vec::new();

    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "has_issues",
        current_settings.has_issues,
        current_settings_value_bool(&current_settings, "has_issues"),
        desired_setting_bool(desired, "has_issues"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "has_projects",
        current_settings.has_projects,
        current_settings_value_bool(&current_settings, "has_projects"),
        desired_setting_bool(desired, "has_projects"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "has_wiki",
        current_settings.has_wiki,
        current_settings_value_bool(&current_settings, "has_wiki"),
        desired_setting_bool(desired, "has_wiki"),
        false,
    );

    let current_discussions = current
        .extensions
        .has_discussions_enabled
        .or(current_settings.has_discussions);
    let desired_discussions = desired_setting_bool(desired, "has_discussions");
    plan_graphql_bool_change(
        &mut changes,
        &mut blocked_changes,
        &mut graphql_patch.has_discussions_enabled,
        "has_discussions",
        current_discussions,
        desired_discussions,
        current.extensions.graphql_settings_collected,
    );

    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "has_pull_requests",
        current.extensions.has_pull_requests,
        current.extensions.has_pull_requests,
        desired_setting_bool(desired, "has_pull_requests").or(desired.extensions.has_pull_requests),
        false,
    );
    plan_policy_change(
        &mut changes,
        &mut rest_patch,
        "pull_request_creation_policy",
        current.extensions.pull_request_creation_policy.clone(),
        desired_setting_string(desired, "pull_request_creation_policy")
            .or_else(|| desired.extensions.pull_request_creation_policy.clone()),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "allow_squash_merge",
        current_settings.allow_squash_merge,
        current_settings_value_bool(&current_settings, "allow_squash_merge"),
        desired_setting_bool(desired, "allow_squash_merge"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "allow_merge_commit",
        current_settings.allow_merge_commit,
        current_settings_value_bool(&current_settings, "allow_merge_commit"),
        desired_setting_bool(desired, "allow_merge_commit"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "allow_rebase_merge",
        current_settings.allow_rebase_merge,
        current_settings_value_bool(&current_settings, "allow_rebase_merge"),
        desired_setting_bool(desired, "allow_rebase_merge"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "allow_auto_merge",
        current_settings.allow_auto_merge,
        current_settings_value_bool(&current_settings, "allow_auto_merge"),
        desired_setting_bool(desired, "allow_auto_merge"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "delete_branch_on_merge",
        current_settings.delete_branch_on_merge,
        current_settings_value_bool(&current_settings, "delete_branch_on_merge"),
        desired_setting_bool(desired, "delete_branch_on_merge"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "allow_update_branch",
        current_settings.allow_update_branch,
        current_settings_value_bool(&current_settings, "allow_update_branch"),
        desired_setting_bool(desired, "allow_update_branch"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "use_squash_pr_title_as_default",
        current.extensions.use_squash_pr_title_as_default,
        current.extensions.use_squash_pr_title_as_default,
        desired_setting_bool(desired, "use_squash_pr_title_as_default")
            .or(desired.extensions.use_squash_pr_title_as_default),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "squash_merge_commit_title",
        current_settings.squash_merge_commit_title.clone(),
        desired_setting_string(desired, "squash_merge_commit_title"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "squash_merge_commit_message",
        current_settings.squash_merge_commit_message.clone(),
        desired_setting_string(desired, "squash_merge_commit_message"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "merge_commit_title",
        current_settings.merge_commit_title.clone(),
        desired_setting_string(desired, "merge_commit_title"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "merge_commit_message",
        current_settings.merge_commit_message.clone(),
        desired_setting_string(desired, "merge_commit_message"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "web_commit_signoff_required",
        current_settings.web_commit_signoff_required,
        current_settings_value_bool(&current_settings, "web_commit_signoff_required"),
        desired_setting_bool(desired, "web_commit_signoff_required"),
        false,
    );
    plan_graphql_bool_change(
        &mut changes,
        &mut blocked_changes,
        &mut graphql_patch.has_sponsorships_enabled,
        "has_sponsorships_enabled",
        current.extensions.has_sponsorships_enabled,
        desired_setting_bool(desired, "has_sponsorships_enabled")
            .or(desired.extensions.has_sponsorships_enabled),
        current.extensions.graphql_settings_collected,
    );
    plan_graphql_policy_change(
        &mut changes,
        &mut blocked_changes,
        &mut graphql_patch.issue_creation_policy,
        "issue_creation_policy",
        current.extensions.issue_creation_policy.clone(),
        desired_setting_string(desired, "issue_creation_policy")
            .or_else(|| desired.extensions.issue_creation_policy.clone()),
        current.extensions.graphql_settings_collected,
    );

    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "description",
        current_metadata.description.clone(),
        desired_metadata_string(desired, "description"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "homepage",
        current_metadata.homepage.clone(),
        desired_metadata_string(desired, "homepage"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "default_branch",
        current_metadata.default_branch.clone(),
        desired_metadata_string(desired, "default_branch"),
        false,
    );
    plan_high_impact_string_change(
        &mut changes,
        &mut blocked_changes,
        &mut rest_patch,
        "visibility",
        current_metadata.visibility.clone(),
        desired_metadata_string(desired, "visibility"),
        allow_high_impact,
    );
    plan_high_impact_bool_change(
        &mut changes,
        &mut blocked_changes,
        &mut rest_patch,
        "archived",
        current_metadata.archived,
        desired_metadata_bool(desired, "archived"),
        allow_high_impact,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "is_template",
        current_metadata.is_template,
        current_metadata.is_template,
        desired_metadata_bool(desired, "is_template"),
        false,
    );
    plan_bool_change(
        &mut changes,
        &mut rest_patch,
        "allow_forking",
        current_metadata.allow_forking,
        current_metadata.allow_forking,
        desired_metadata_bool(desired, "allow_forking"),
        false,
    );

    let topics = planned_topics(desired, current, &mut changes, &mut blocked_changes);
    let custom_property_updates =
        planned_custom_property_updates(desired, current, &mut changes, &mut blocked_changes);
    let immutable_releases =
        planned_immutable_release_action(desired, current, &mut changes, &mut blocked_changes);
    let label_actions = planned_label_actions(desired, current, &mut changes, &mut blocked_changes);

    GeneralPlan {
        repo: repo.to_owned(),
        repository_id: current.extensions.repository_id.clone(),
        desired: desired.clone(),
        options,
        coverage: current.coverage.clone(),
        changes,
        blocked_changes,
        rest_patch: Value::Object(rest_patch),
        graphql_patch: (!graphql_patch.is_empty()).then_some(graphql_patch),
        topics,
        custom_property_updates,
        immutable_releases,
        label_actions,
    }
}

pub async fn apply(client: &Client, plan: &GeneralPlan) -> Result<GeneralVerification> {
    if plan.has_blocked_changes() {
        bail!(
            "General settings plan for {} is blocked by {} change(s)",
            plan.repo,
            plan.blocked_changes.len()
        );
    }

    if let Some(branch) = plan
        .rest_patch
        .as_object()
        .and_then(|body| body.get("default_branch"))
        .and_then(Value::as_str)
        && !client.branch_exists(&plan.repo, branch).await?
    {
        bail!(
            "Cannot set default branch for {} to {branch}: branch does not exist",
            plan.repo
        );
    }

    if plan
        .rest_patch
        .as_object()
        .is_some_and(|body| !body.is_empty())
    {
        client.update_settings(&plan.repo, &plan.rest_patch).await?;
    }

    if let Some(graphql_patch) = plan.graphql_patch.as_ref() {
        client
            .update_repository_graphql_settings(&plan.repository_id, graphql_patch)
            .await?;
    }

    if let Some(topics) = plan.topics.as_ref() {
        client.replace_topics(&plan.repo, topics).await?;
    }

    if !plan.custom_property_updates.is_empty() {
        let updates = plan
            .custom_property_updates
            .iter()
            .cloned()
            .map(|update| CustomPropertyValueMutation {
                property_name: update.property_name,
                value: update.value,
            })
            .collect::<Vec<_>>();
        match client
            .update_custom_property_values(&plan.repo, &updates)
            .await?
        {
            ClassifiedApiResponse::Success(()) | ClassifiedApiResponse::NoContent => {}
            ClassifiedApiResponse::Forbidden(message)
            | ClassifiedApiResponse::NotFound(message)
            | ClassifiedApiResponse::Unprocessable(message)
            | ClassifiedApiResponse::Conflict(message)
            | ClassifiedApiResponse::Other(message) => {
                bail!(
                    "Failed to update custom properties for {}: {message}",
                    plan.repo
                );
            }
        }
    }

    match plan.immutable_releases.as_ref() {
        Some(PlannedImmutableReleaseAction::Enable) => {
            match client.enable_immutable_releases(&plan.repo).await? {
                ClassifiedApiResponse::Success(()) | ClassifiedApiResponse::NoContent => {}
                ClassifiedApiResponse::Forbidden(message)
                | ClassifiedApiResponse::NotFound(message)
                | ClassifiedApiResponse::Unprocessable(message)
                | ClassifiedApiResponse::Conflict(message)
                | ClassifiedApiResponse::Other(message) => {
                    bail!(
                        "Failed to enable immutable releases for {}: {message}",
                        plan.repo
                    );
                }
            }
        }
        Some(PlannedImmutableReleaseAction::Disable) => {
            match client.disable_immutable_releases(&plan.repo).await? {
                ClassifiedApiResponse::Success(()) | ClassifiedApiResponse::NoContent => {}
                ClassifiedApiResponse::Forbidden(message)
                | ClassifiedApiResponse::NotFound(message)
                | ClassifiedApiResponse::Unprocessable(message)
                | ClassifiedApiResponse::Conflict(message)
                | ClassifiedApiResponse::Other(message) => {
                    bail!(
                        "Failed to disable immutable releases for {}: {message}",
                        plan.repo
                    );
                }
            }
        }
        Some(PlannedImmutableReleaseAction::Reference) | None => {}
    }

    for action in &plan.label_actions {
        match action {
            PlannedLabelAction::Create { label } => {
                let color = label.color.as_deref().with_context(|| {
                    format!("Cannot create label {} without a color", label.name)
                })?;
                client
                    .create_label(
                        &plan.repo,
                        &label.name,
                        &normalize_label_color(color),
                        label.description.as_deref(),
                    )
                    .await?;
            }
            PlannedLabelAction::Update {
                current_name,
                label,
            } => {
                let normalized_color = label.color.as_deref().map(normalize_label_color);
                client
                    .update_label(
                        &plan.repo,
                        current_name,
                        None,
                        normalized_color.as_deref(),
                        label.description.as_deref(),
                    )
                    .await?;
            }
            PlannedLabelAction::Delete { name } => {
                client.delete_label(&plan.repo, name).await?;
            }
        }
    }

    let verification = verify_with_options(client, &plan.repo, &plan.desired, plan.options).await?;
    if !verification.compliant {
        bail!(
            "General settings verification failed for {}: {} remaining change(s), {} blocked change(s)",
            plan.repo,
            verification.remaining_changes.len(),
            verification.blocked_changes.len()
        );
    }

    Ok(verification)
}

pub async fn verify(
    client: &Client,
    repo: &str,
    desired: &GeneralDesiredState,
) -> Result<GeneralVerification> {
    verify_with_options(client, repo, desired, GeneralPlanOptions::default()).await
}

pub async fn verify_with_options(
    client: &Client,
    repo: &str,
    desired: &GeneralDesiredState,
    options: GeneralPlanOptions,
) -> Result<GeneralVerification> {
    let current = collect(client, repo).await?;
    let planned = plan_with_options(repo, desired, &current, options);
    let compliant = !planned.has_actionable_changes() && !planned.has_blocked_changes();

    Ok(GeneralVerification {
        repo: repo.to_owned(),
        compliant,
        coverage: current.coverage,
        remaining_changes: planned.changes,
        blocked_changes: planned.blocked_changes,
    })
}

fn build_repository_settings(
    rest: &RepositoryGeneralSettings,
    topics: Option<&Vec<String>>,
    graphql: Option<&GraphqlRepositorySettings>,
) -> Result<RepositorySettingsConfig> {
    let mut map = Map::new();
    insert_value(&mut map, "has_issues", json!(rest.has_issues));
    insert_value(&mut map, "has_projects", json!(rest.has_projects));
    insert_value(&mut map, "has_wiki", json!(rest.has_wiki));
    insert_value(
        &mut map,
        "has_discussions",
        json!(graphql.map_or(rest.has_discussions, |value| value.has_discussions_enabled)),
    );
    insert_value(&mut map, "has_pull_requests", json!(rest.has_pull_requests));
    if let Some(value) = normalize_optional_policy(rest.pull_request_creation_policy.as_deref()) {
        insert_value(&mut map, "pull_request_creation_policy", json!(value));
    }
    insert_value(
        &mut map,
        "allow_squash_merge",
        json!(rest.allow_squash_merge),
    );
    insert_value(
        &mut map,
        "allow_merge_commit",
        json!(rest.allow_merge_commit),
    );
    insert_value(
        &mut map,
        "allow_rebase_merge",
        json!(rest.allow_rebase_merge),
    );
    insert_value(&mut map, "allow_auto_merge", json!(rest.allow_auto_merge));
    insert_value(
        &mut map,
        "delete_branch_on_merge",
        json!(rest.delete_branch_on_merge),
    );
    insert_value(
        &mut map,
        "allow_update_branch",
        json!(rest.allow_update_branch),
    );
    if let Some(value) = rest.use_squash_pr_title_as_default {
        insert_value(&mut map, "use_squash_pr_title_as_default", json!(value));
    }
    if let Some(value) = rest.squash_merge_commit_title.as_ref() {
        insert_value(&mut map, "squash_merge_commit_title", json!(value));
    }
    if let Some(value) = rest.squash_merge_commit_message.as_ref() {
        insert_value(&mut map, "squash_merge_commit_message", json!(value));
    }
    if let Some(value) = rest.merge_commit_title.as_ref() {
        insert_value(&mut map, "merge_commit_title", json!(value));
    }
    if let Some(value) = rest.merge_commit_message.as_ref() {
        insert_value(&mut map, "merge_commit_message", json!(value));
    }
    insert_value(
        &mut map,
        "web_commit_signoff_required",
        json!(rest.web_commit_signoff_required),
    );
    if let Some(values) = topics {
        insert_value(&mut map, "topics", json!(values));
    }
    if let Some(graphql) = graphql {
        insert_value(
            &mut map,
            "has_sponsorships_enabled",
            json!(graphql.has_sponsorships_enabled),
        );
        if let Some(value) = normalize_optional_policy(graphql.issue_creation_policy.as_deref()) {
            insert_value(&mut map, "issue_creation_policy", json!(value));
        }
    }

    serde_json::from_value(Value::Object(map))
        .context("Failed to build repository settings snapshot from collected state")
}

fn build_repository_metadata(rest: &RepositoryGeneralSettings) -> Result<RepositoryMetadataConfig> {
    let mut map = Map::new();
    insert_value(
        &mut map,
        "description",
        json!(rest.description.clone().unwrap_or_default()),
    );
    insert_value(
        &mut map,
        "homepage",
        json!(rest.homepage.clone().unwrap_or_default()),
    );
    insert_value(&mut map, "default_branch", json!(rest.default_branch));
    insert_value(&mut map, "visibility", json!(rest.visibility));
    insert_value(&mut map, "archived", json!(rest.archived));
    insert_value(&mut map, "is_template", json!(rest.is_template));
    insert_value(&mut map, "allow_forking", json!(rest.allow_forking));

    serde_json::from_value(Value::Object(map))
        .context("Failed to build repository metadata snapshot from collected state")
}

fn collect_custom_properties(
    values: &[RepositoryCustomPropertyValue],
) -> Vec<GeneralCustomPropertyValue> {
    let mut collected = values
        .iter()
        .map(|value| GeneralCustomPropertyValue {
            property_name: value.property_name.clone(),
            value: value.value.clone(),
        })
        .collect::<Vec<_>>();
    collected.sort_by(|left, right| left.property_name.cmp(&right.property_name));
    collected
}

fn collect_labels(labels: Vec<RepositoryLabel>) -> Vec<GeneralLabel> {
    let mut collected = labels
        .into_iter()
        .map(|label| GeneralLabel {
            name: label.name,
            color: Some(normalize_label_color(&label.color)),
            description: label.description,
            default: label.default,
        })
        .collect::<Vec<_>>();
    collected.sort_by(|left, right| left.name.cmp(&right.name));
    collected
}

fn manifest_compatible_custom_properties(
    values: &[GeneralCustomPropertyValue],
) -> Vec<CustomPropertyValueConfig> {
    values
        .iter()
        .filter_map(|value| {
            serde_json::from_value::<CustomPropertyValueConfig>(json!({
                "property_name": value.property_name,
                "value": value.value,
            }))
            .ok()
        })
        .collect()
}

fn planned_topics(
    desired: &GeneralDesiredState,
    current: &CollectedGeneralState,
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
) -> Option<Vec<String>> {
    let desired_topics = desired_topics(desired)?;
    let Some(current_topics) = current
        .repository
        .settings
        .as_ref()
        .and_then(|settings| settings.topics.clone())
    else {
        if !desired_topics.is_empty() || desired.repository.policy.prune {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::Topics,
                "<unavailable>",
                format_topics(&desired_topics),
                "Current topics could not be collected, so replacing topics would be unsafe",
                false,
            ));
        }
        return None;
    };
    let current_topics = normalize_topics(&current_topics);

    if desired.repository.policy.prune {
        let desired_topics = normalize_topics(&desired_topics);
        if topic_set(&current_topics) != topic_set(&desired_topics) {
            changes.push(GeneralChange {
                kind: GeneralChangeKind::Topics,
                current: format_topics(&current_topics),
                desired: format_topics(&desired_topics),
                high_impact: false,
                reference_only: false,
                reason: None,
            });
            Some(desired_topics)
        } else {
            None
        }
    } else {
        let desired_topics = normalize_topics(&desired_topics);
        let target = union_topics(&current_topics, &desired_topics);
        if topic_set(&current_topics) != topic_set(&target) {
            changes.push(GeneralChange {
                kind: GeneralChangeKind::Topics,
                current: format_topics(&current_topics),
                desired: format_topics(&target),
                high_impact: false,
                reference_only: false,
                reason: None,
            });
            Some(target)
        } else {
            None
        }
    }
}

fn planned_custom_property_updates(
    desired: &GeneralDesiredState,
    current: &CollectedGeneralState,
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
) -> Vec<PlannedCustomPropertyUpdate> {
    let desired_values = desired_custom_properties(desired);
    if desired_values.is_empty() && !desired.repository.policy.prune {
        return Vec::new();
    }
    if !current.extensions.custom_properties_collected {
        for property in &desired_values {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::CustomProperty {
                    property_name: property.property_name.clone(),
                    action: GeneralResourceAction::Update,
                },
                "<unavailable>",
                display_json_value(&property.value),
                "Current custom properties could not be collected",
                false,
            ));
        }
        if desired.repository.policy.prune {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::CustomProperty {
                    property_name: "*".to_owned(),
                    action: GeneralResourceAction::Delete,
                },
                "<unavailable>",
                "<prune>".to_owned(),
                "Current custom properties could not be collected, so prune is unsafe",
                false,
            ));
        }
        return Vec::new();
    }

    let current_values = current_custom_properties(current);
    let current_map = current_values
        .iter()
        .map(|property| (property.property_name.clone(), property.value.clone()))
        .collect::<BTreeMap<_, _>>();
    let desired_map = desired_values
        .iter()
        .map(|property| (property.property_name.clone(), property.value.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut updates = Vec::new();
    for (property_name, desired_value) in &desired_map {
        if current_map.get(property_name) != Some(desired_value) {
            changes.push(GeneralChange {
                kind: GeneralChangeKind::CustomProperty {
                    property_name: property_name.clone(),
                    action: if current_map.contains_key(property_name) {
                        GeneralResourceAction::Update
                    } else {
                        GeneralResourceAction::Create
                    },
                },
                current: current_map
                    .get(property_name)
                    .map(display_json_value)
                    .unwrap_or_else(|| "<unset>".to_owned()),
                desired: display_json_value(desired_value),
                high_impact: false,
                reference_only: false,
                reason: None,
            });
            updates.push(PlannedCustomPropertyUpdate {
                property_name: property_name.clone(),
                value: Some(desired_value.clone()),
            });
        }
    }

    if desired.repository.policy.prune {
        for (property_name, current_value) in &current_map {
            if !desired_map.contains_key(property_name) {
                changes.push(GeneralChange {
                    kind: GeneralChangeKind::CustomProperty {
                        property_name: property_name.clone(),
                        action: GeneralResourceAction::Delete,
                    },
                    current: display_json_value(current_value),
                    desired: "<unset>".to_owned(),
                    high_impact: false,
                    reference_only: false,
                    reason: None,
                });
                updates.push(PlannedCustomPropertyUpdate {
                    property_name: property_name.clone(),
                    value: None,
                });
            }
        }
    }

    updates
}

fn planned_immutable_release_action(
    desired: &GeneralDesiredState,
    current: &CollectedGeneralState,
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
) -> Option<PlannedImmutableReleaseAction> {
    let desired_immutable = desired.repository.immutable_releases.as_ref()?;
    let Some(current_immutable) = current.repository.immutable_releases.clone() else {
        blocked_changes.push(blocked_change(
            GeneralChangeKind::ImmutableReleases {
                action: GeneralResourceAction::Reference,
            },
            "<unavailable>",
            display_immutable_releases(desired_immutable),
            "Immutable releases state could not be collected",
            false,
        ));
        return None;
    };
    let current_enabled = current_immutable.enabled.unwrap_or(false);
    let enforced_by_owner = current_immutable.enforced_by_owner.unwrap_or(false);

    if let Some(desired_enabled) = desired_immutable.enabled
        && current_enabled != desired_enabled
    {
        if enforced_by_owner && !desired_enabled {
            blocked_changes.push(GeneralChange {
                kind: GeneralChangeKind::ImmutableReleases {
                    action: GeneralResourceAction::Reference,
                },
                current: "enabled (owner-enforced)".to_owned(),
                desired: "disabled".to_owned(),
                high_impact: false,
                reference_only: true,
                reason: Some(
                    "Immutable releases are enforced by the owner and cannot be disabled here"
                        .to_owned(),
                ),
            });
            return Some(PlannedImmutableReleaseAction::Reference);
        }

        changes.push(GeneralChange {
            kind: GeneralChangeKind::ImmutableReleases {
                action: if desired_enabled {
                    GeneralResourceAction::Enable
                } else {
                    GeneralResourceAction::Disable
                },
            },
            current: current_enabled.to_string(),
            desired: desired_enabled.to_string(),
            high_impact: false,
            reference_only: false,
            reason: None,
        });
        return Some(if desired_enabled {
            PlannedImmutableReleaseAction::Enable
        } else {
            PlannedImmutableReleaseAction::Disable
        });
    }

    if let Some(desired_enforced) = desired_immutable.enforced_by_owner
        && enforced_by_owner != desired_enforced
    {
        blocked_changes.push(GeneralChange {
            kind: GeneralChangeKind::ImmutableReleases {
                action: GeneralResourceAction::Reference,
            },
            current: enforced_by_owner.to_string(),
            desired: desired_enforced.to_string(),
            high_impact: false,
            reference_only: true,
            reason: Some("Owner enforcement is read-only at repository scope".to_owned()),
        });
        return Some(PlannedImmutableReleaseAction::Reference);
    }

    None
}

fn planned_label_actions(
    desired: &GeneralDesiredState,
    current: &CollectedGeneralState,
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
) -> Vec<PlannedLabelAction> {
    let desired_labels = desired.labels.clone();
    if desired_labels.is_empty() && !desired.repository.policy.prune {
        return Vec::new();
    }
    if !current.extensions.labels_collected {
        for label in &desired_labels {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::Label {
                    name: label.name.clone(),
                    action: GeneralResourceAction::Update,
                },
                "<unavailable>",
                format!("{label:?}"),
                "Current labels could not be collected",
                false,
            ));
        }
        if desired.repository.policy.prune {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::Label {
                    name: "*".to_owned(),
                    action: GeneralResourceAction::Delete,
                },
                "<unavailable>",
                "<prune>".to_owned(),
                "Current labels could not be collected, so prune is unsafe",
                false,
            ));
        }
        return Vec::new();
    }

    let current_labels = current
        .labels
        .iter()
        .cloned()
        .map(|label| (label.name.clone(), label))
        .collect::<BTreeMap<_, _>>();
    let desired_labels = desired_labels
        .into_iter()
        .map(|label| (label.name.clone(), label))
        .collect::<BTreeMap<_, _>>();

    let mut actions = Vec::new();
    for (name, desired_label) in &desired_labels {
        match current_labels.get(name) {
            None => {
                changes.push(GeneralChange {
                    kind: GeneralChangeKind::Label {
                        name: name.clone(),
                        action: GeneralResourceAction::Create,
                    },
                    current: "<missing>".to_owned(),
                    desired: format!("{desired_label:?}"),
                    high_impact: false,
                    reference_only: false,
                    reason: None,
                });
                actions.push(PlannedLabelAction::Create {
                    label: desired_label.clone(),
                });
            }
            Some(current_label) if label_needs_update(current_label, desired_label) => {
                changes.push(GeneralChange {
                    kind: GeneralChangeKind::Label {
                        name: name.clone(),
                        action: GeneralResourceAction::Update,
                    },
                    current: format!("{current_label:?}"),
                    desired: format!("{desired_label:?}"),
                    high_impact: false,
                    reference_only: false,
                    reason: None,
                });
                actions.push(PlannedLabelAction::Update {
                    current_name: name.clone(),
                    label: desired_label.clone(),
                });
            }
            Some(_) => {}
        }
    }

    if desired.repository.policy.prune {
        for (name, current_label) in &current_labels {
            if desired_labels.contains_key(name) {
                continue;
            }
            if current_label.default {
                blocked_changes.push(blocked_change(
                    GeneralChangeKind::Label {
                        name: name.clone(),
                        action: GeneralResourceAction::Delete,
                    },
                    format!("{current_label:?}"),
                    "<unset>".to_owned(),
                    "GitHub default labels are not pruned automatically",
                    false,
                ));
                continue;
            }
            changes.push(GeneralChange {
                kind: GeneralChangeKind::Label {
                    name: name.clone(),
                    action: GeneralResourceAction::Delete,
                },
                current: format!("{current_label:?}"),
                desired: "<unset>".to_owned(),
                high_impact: false,
                reference_only: false,
                reason: None,
            });
            actions.push(PlannedLabelAction::Delete { name: name.clone() });
        }
    }

    actions
}

fn label_needs_update(current: &GeneralLabel, desired: &GeneralLabel) -> bool {
    if let Some(desired_color) = desired.color.as_deref()
        && current.color.as_deref().map(normalize_label_color)
            != Some(normalize_label_color(desired_color))
    {
        return true;
    }
    if let Some(desired_description) = desired.description.as_deref()
        && current.description.as_deref() != Some(desired_description)
    {
        return true;
    }
    false
}

fn desired_topics(desired: &GeneralDesiredState) -> Option<Vec<String>> {
    let value = desired_setting_value(desired, "topics")?;
    let values = value
        .as_array()?
        .iter()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    Some(normalize_topics(&values))
}

fn current_custom_properties(current: &CollectedGeneralState) -> Vec<GeneralCustomPropertyValue> {
    if !current.custom_properties.is_empty() {
        return current.custom_properties.clone();
    }
    extract_manifest_custom_properties(&current.repository)
}

fn desired_custom_properties(desired: &GeneralDesiredState) -> Vec<GeneralCustomPropertyValue> {
    if !desired.custom_properties.is_empty() {
        return desired.custom_properties.clone();
    }
    extract_manifest_custom_properties(&desired.repository)
}

fn extract_manifest_custom_properties(
    repository: &RepositoryCategoryV2,
) -> Vec<GeneralCustomPropertyValue> {
    let Some(Value::Array(values)) = repository_to_value(repository)
        .ok()
        .and_then(|value| value.get("custom_properties").cloned())
    else {
        return repository
            .custom_properties
            .iter()
            .map(|property| GeneralCustomPropertyValue {
                property_name: property.property_name.clone(),
                value: json!(property.value),
            })
            .collect();
    };

    values
        .into_iter()
        .filter_map(|value| {
            let property_name = value.get("property_name")?.as_str()?.to_owned();
            Some(GeneralCustomPropertyValue {
                property_name,
                value: value.get("value").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

fn desired_setting_value(desired: &GeneralDesiredState, field: &str) -> Option<Value> {
    settings_to_value(desired.repository.settings.as_ref())
        .and_then(|value| value.get(field).cloned())
}

fn desired_setting_bool(desired: &GeneralDesiredState, field: &str) -> Option<bool> {
    desired_setting_value(desired, field).and_then(|value| value.as_bool())
}

fn desired_setting_string(desired: &GeneralDesiredState, field: &str) -> Option<String> {
    normalize_optional_value(desired_setting_value(desired, field))
}

fn desired_metadata_bool(desired: &GeneralDesiredState, field: &str) -> Option<bool> {
    metadata_to_value(desired.repository.metadata.as_ref())
        .and_then(|value| value.get(field)?.as_bool())
}

fn desired_metadata_string(desired: &GeneralDesiredState, field: &str) -> Option<String> {
    normalize_optional_value(
        metadata_to_value(desired.repository.metadata.as_ref())
            .and_then(|value| value.get(field).cloned()),
    )
}

fn current_settings_value_bool(settings: &RepositorySettingsConfig, field: &str) -> Option<bool> {
    settings_to_value(Some(settings)).and_then(|value| value.get(field)?.as_bool())
}

fn settings_to_value(settings: Option<&RepositorySettingsConfig>) -> Option<Map<String, Value>> {
    let value = serde_json::to_value(settings?).ok()?;
    value.as_object().cloned()
}

fn metadata_to_value(metadata: Option<&RepositoryMetadataConfig>) -> Option<Map<String, Value>> {
    let value = serde_json::to_value(metadata?).ok()?;
    value.as_object().cloned()
}

fn repository_to_value(repository: &RepositoryCategoryV2) -> Result<Map<String, Value>> {
    let value = serde_json::to_value(repository)?;
    value
        .as_object()
        .cloned()
        .context("RepositoryCategoryV2 did not serialize to an object")
}

fn normalize_optional_value(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value),
        Some(Value::Null) => Some(String::new()),
        _ => None,
    }
}

fn normalize_label_color(value: &str) -> String {
    value.trim_start_matches('#').to_ascii_lowercase()
}

fn normalize_topics(topics: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for topic in topics {
        let topic = topic.trim().to_owned();
        if !topic.is_empty() && seen.insert(topic.clone()) {
            normalized.push(topic);
        }
    }
    normalized
}

fn union_topics(current: &[String], desired: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut topics = Vec::new();
    for topic in current.iter().chain(desired.iter()) {
        if seen.insert(topic.clone()) {
            topics.push(topic.clone());
        }
    }
    topics
}

fn topic_set(topics: &[String]) -> BTreeSet<String> {
    topics.iter().cloned().collect()
}

fn plan_bool_change(
    changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_display_value: Option<bool>,
    current_value: Option<bool>,
    desired_value: Option<bool>,
    high_impact: bool,
) {
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value)
    {
        rest_patch.insert(field.to_owned(), json!(desired_value));
        changes.push(GeneralChange {
            kind: GeneralChangeKind::RestField {
                field: field.to_owned(),
            },
            current: current_display_value
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<unset>".to_owned()),
            desired: desired_value.to_string(),
            high_impact,
            reference_only: false,
            reason: None,
        });
    }
}

fn plan_optional_string_change(
    changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_value: Option<String>,
    desired_value: Option<String>,
    high_impact: bool,
) {
    if let Some(desired_value) = desired_value
        && current_value.as_deref() != Some(desired_value.as_str())
    {
        rest_patch.insert(field.to_owned(), json!(desired_value.clone()));
        changes.push(GeneralChange {
            kind: GeneralChangeKind::RestField {
                field: field.to_owned(),
            },
            current: display_optional_string(current_value.as_deref()),
            desired: display_optional_string(Some(desired_value.as_str())),
            high_impact,
            reference_only: false,
            reason: None,
        });
    }
}

fn plan_policy_change(
    changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_value: Option<String>,
    desired_value: Option<String>,
    high_impact: bool,
) {
    let desired_value = normalize_optional_policy(desired_value.as_deref());
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value.clone())
    {
        rest_patch.insert(field.to_owned(), json!(desired_value.clone()));
        changes.push(GeneralChange {
            kind: GeneralChangeKind::RestField {
                field: field.to_owned(),
            },
            current: current_value.unwrap_or_else(|| "<unset>".to_owned()),
            desired: desired_value,
            high_impact,
            reference_only: false,
            reason: None,
        });
    }
}

fn plan_graphql_bool_change(
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
    patch_field: &mut Option<bool>,
    field: &str,
    current_value: Option<bool>,
    desired_value: Option<bool>,
    graphql_collected: bool,
) {
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value)
    {
        if !graphql_collected {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::GraphqlField {
                    field: field.to_owned(),
                },
                current_value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<unavailable>".to_owned()),
                desired_value.to_string(),
                "GraphQL repository settings could not be collected",
                false,
            ));
            return;
        }

        *patch_field = Some(desired_value);
        changes.push(GeneralChange {
            kind: GeneralChangeKind::GraphqlField {
                field: field.to_owned(),
            },
            current: current_value
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<unset>".to_owned()),
            desired: desired_value.to_string(),
            high_impact: false,
            reference_only: false,
            reason: None,
        });
    }
}

fn plan_graphql_policy_change(
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
    patch_field: &mut Option<String>,
    field: &str,
    current_value: Option<String>,
    desired_value: Option<String>,
    graphql_collected: bool,
) {
    let desired_value = normalize_optional_policy(desired_value.as_deref());
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value.clone())
    {
        if !graphql_collected {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::GraphqlField {
                    field: field.to_owned(),
                },
                current_value.unwrap_or_else(|| "<unavailable>".to_owned()),
                desired_value,
                "GraphQL repository settings could not be collected",
                false,
            ));
            return;
        }

        *patch_field = Some(desired_value.clone());
        changes.push(GeneralChange {
            kind: GeneralChangeKind::GraphqlField {
                field: field.to_owned(),
            },
            current: current_value.unwrap_or_else(|| "<unset>".to_owned()),
            desired: desired_value,
            high_impact: false,
            reference_only: false,
            reason: None,
        });
    }
}

fn plan_high_impact_bool_change(
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_value: Option<bool>,
    desired_value: Option<bool>,
    allow_high_impact: bool,
) {
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value)
    {
        if allow_high_impact {
            rest_patch.insert(field.to_owned(), json!(desired_value));
            changes.push(GeneralChange {
                kind: GeneralChangeKind::RestField {
                    field: field.to_owned(),
                },
                current: current_value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<unset>".to_owned()),
                desired: desired_value.to_string(),
                high_impact: true,
                reference_only: false,
                reason: None,
            });
        } else {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::RestField {
                    field: field.to_owned(),
                },
                current_value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<unset>".to_owned()),
                desired_value.to_string(),
                "High-impact repository changes require allow_high_impact or a sensitive policy opt-in",
                true,
            ));
        }
    }
}

fn plan_high_impact_string_change(
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_value: Option<String>,
    desired_value: Option<String>,
    allow_high_impact: bool,
) {
    if let Some(desired_value) = desired_value
        && current_value.as_deref() != Some(desired_value.as_str())
    {
        if allow_high_impact {
            rest_patch.insert(field.to_owned(), json!(desired_value.clone()));
            changes.push(GeneralChange {
                kind: GeneralChangeKind::RestField {
                    field: field.to_owned(),
                },
                current: display_optional_string(current_value.as_deref()),
                desired: display_optional_string(Some(desired_value.as_str())),
                high_impact: true,
                reference_only: false,
                reason: None,
            });
        } else {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::RestField {
                    field: field.to_owned(),
                },
                display_optional_string(current_value.as_deref()),
                display_optional_string(Some(desired_value.as_str())),
                "High-impact repository changes require allow_high_impact or a sensitive policy opt-in",
                true,
            ));
        }
    }
}

fn blocked_change(
    kind: GeneralChangeKind,
    current: impl Into<String>,
    desired: impl Into<String>,
    reason: impl Into<String>,
    high_impact: bool,
) -> GeneralChange {
    GeneralChange {
        kind,
        current: current.into(),
        desired: desired.into(),
        high_impact,
        reference_only: true,
        reason: Some(reason.into()),
    }
}

fn display_optional_string(value: Option<&str>) -> String {
    match value {
        Some("") => "<empty>".to_owned(),
        Some(value) => value.to_owned(),
        None => "<unset>".to_owned(),
    }
}

fn display_json_value(value: &Value) -> String {
    match value {
        Value::Null => "<unset>".to_owned(),
        Value::String(value) if value.is_empty() => "<empty>".to_owned(),
        Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<invalid json>".to_owned()),
    }
}

fn display_immutable_releases(value: &ImmutableReleasesConfig) -> String {
    format!(
        "enabled={}, enforced_by_owner={}",
        value
            .enabled
            .map(|enabled| enabled.to_string())
            .unwrap_or_else(|| "<unset>".to_owned()),
        value
            .enforced_by_owner
            .map(|enabled| enabled.to_string())
            .unwrap_or_else(|| "<unset>".to_owned())
    )
}

fn format_topics(topics: &[String]) -> String {
    format!("{topics:?}")
}

fn insert_value(map: &mut Map<String, Value>, field: &str, value: Value) {
    map.insert(field.to_owned(), value);
}

fn has_coverage_entry(coverage: &[CoverageEntry], endpoint: &str) -> bool {
    coverage.iter().any(|entry| entry.endpoint == endpoint)
}

fn normalize_optional_policy(value: Option<&str>) -> Option<String> {
    value.map(|value| value.trim().replace(['-', ' '], "_").to_ascii_lowercase())
}

fn unsupported_repository_settings_coverage() -> Vec<CoverageEntry> {
    [
        "commit comments",
        "LFS archives",
        "multi-ref push limit",
        "auto-close linked issues",
    ]
    .into_iter()
    .map(|setting| {
        coverage_entry(
            ManifestCategoryName::Repository,
            &format!("Repository settings UI: {setting}"),
            CoverageOutcome::Unsupported,
            Some(
                "GitHub OpenAPI and GraphQL do not expose an official mutation for this setting"
                    .to_owned(),
            ),
            None,
        )
    })
    .collect()
}

fn coverage_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    outcome: CoverageOutcome,
    reason: Option<String>,
    required_permission: Option<String>,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome,
        reason,
        required_permission,
    }
}
