//! Planning of repository setting and metadata changes.

use super::resources::{
    planned_custom_property_updates, planned_immutable_release_action, planned_label_actions,
    planned_topics,
};
use super::{
    CollectedGeneralState, GeneralChange, GeneralChangeKind, GeneralDesiredState, GeneralPlan,
    GeneralPlanOptions,
};
use crate::config::manifest::{
    ManagementDisposition, RepositoryCategory, RepositoryMetadataConfig, RepositorySettingsConfig,
};
use crate::github::settings::GraphqlRepositoryPatch;
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

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
        current.extensions.pull_request_creation_policy.as_deref(),
        desired_setting_string(desired, "pull_request_creation_policy")
            .or_else(|| desired.extensions.pull_request_creation_policy.clone())
            .as_deref(),
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
        current_settings.squash_merge_commit_title.as_deref(),
        desired_setting_string(desired, "squash_merge_commit_title"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "squash_merge_commit_message",
        current_settings.squash_merge_commit_message.as_deref(),
        desired_setting_string(desired, "squash_merge_commit_message"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "merge_commit_title",
        current_settings.merge_commit_title.as_deref(),
        desired_setting_string(desired, "merge_commit_title"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "merge_commit_message",
        current_settings.merge_commit_message.as_deref(),
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
        current.extensions.issue_creation_policy.as_deref(),
        desired_setting_string(desired, "issue_creation_policy")
            .or_else(|| desired.extensions.issue_creation_policy.clone())
            .as_deref(),
        current.extensions.graphql_settings_collected,
    );

    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "description",
        current_metadata.description.as_deref(),
        desired_metadata_string(desired, "description"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "homepage",
        current_metadata.homepage.as_deref(),
        desired_metadata_string(desired, "homepage"),
        false,
    );
    plan_optional_string_change(
        &mut changes,
        &mut rest_patch,
        "default_branch",
        current_metadata.default_branch.as_deref(),
        desired_metadata_string(desired, "default_branch"),
        false,
    );
    plan_high_impact_string_change(
        &mut changes,
        &mut blocked_changes,
        &mut rest_patch,
        "visibility",
        current_metadata.visibility.as_deref(),
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

pub(super) fn desired_setting_value(desired: &GeneralDesiredState, field: &str) -> Option<Value> {
    settings_to_value(desired.repository.settings.as_ref())
        .and_then(|value| value.get(field).cloned())
}

pub(super) fn desired_setting_bool(desired: &GeneralDesiredState, field: &str) -> Option<bool> {
    desired_setting_value(desired, field).and_then(|value| value.as_bool())
}

pub(super) fn desired_setting_string(desired: &GeneralDesiredState, field: &str) -> Option<String> {
    normalize_optional_value(desired_setting_value(desired, field))
}

pub(super) fn desired_metadata_bool(desired: &GeneralDesiredState, field: &str) -> Option<bool> {
    metadata_to_value(desired.repository.metadata.as_ref())
        .and_then(|value| value.get(field)?.as_bool())
}

pub(super) fn desired_metadata_string(
    desired: &GeneralDesiredState,
    field: &str,
) -> Option<String> {
    normalize_optional_value(
        metadata_to_value(desired.repository.metadata.as_ref())
            .and_then(|value| value.get(field).cloned()),
    )
}

pub(super) fn current_settings_value_bool(
    settings: &RepositorySettingsConfig,
    field: &str,
) -> Option<bool> {
    settings_to_value(Some(settings)).and_then(|value| value.get(field)?.as_bool())
}

pub(super) fn settings_to_value(
    settings: Option<&RepositorySettingsConfig>,
) -> Option<Map<String, Value>> {
    let value = serde_json::to_value(settings?).ok()?;
    value.as_object().cloned()
}

pub(super) fn metadata_to_value(
    metadata: Option<&RepositoryMetadataConfig>,
) -> Option<Map<String, Value>> {
    let value = serde_json::to_value(metadata?).ok()?;
    value.as_object().cloned()
}

pub(super) fn repository_to_value(repository: &RepositoryCategory) -> Result<Map<String, Value>> {
    let value = serde_json::to_value(repository)?;
    value
        .as_object()
        .cloned()
        .context("RepositoryCategory did not serialize to an object")
}

pub(super) fn normalize_optional_value(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value),
        Some(Value::Null) => Some(String::new()),
        _ => None,
    }
}

pub(super) fn plan_bool_change(
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

pub(super) fn plan_optional_string_change(
    changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_value: Option<&str>,
    desired_value: Option<String>,
    high_impact: bool,
) {
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value.as_str())
    {
        rest_patch.insert(field.to_owned(), json!(desired_value));
        changes.push(GeneralChange {
            kind: GeneralChangeKind::RestField {
                field: field.to_owned(),
            },
            current: display_optional_string(current_value),
            desired: display_optional_string(Some(desired_value.as_str())),
            high_impact,
            reference_only: false,
            reason: None,
        });
    }
}

pub(super) fn plan_policy_change(
    changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_value: Option<&str>,
    desired_value: Option<&str>,
    high_impact: bool,
) {
    let desired_value = normalize_optional_policy(desired_value);
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value.as_str())
    {
        rest_patch.insert(field.to_owned(), json!(desired_value));
        changes.push(GeneralChange {
            kind: GeneralChangeKind::RestField {
                field: field.to_owned(),
            },
            current: current_value.unwrap_or("<unset>").to_owned(),
            desired: desired_value,
            high_impact,
            reference_only: false,
            reason: None,
        });
    }
}

pub(super) fn plan_graphql_bool_change(
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

pub(super) fn plan_graphql_policy_change(
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
    patch_field: &mut Option<String>,
    field: &str,
    current_value: Option<&str>,
    desired_value: Option<&str>,
    graphql_collected: bool,
) {
    let desired_value = normalize_optional_policy(desired_value);
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value.as_str())
    {
        if !graphql_collected {
            blocked_changes.push(blocked_change(
                GeneralChangeKind::GraphqlField {
                    field: field.to_owned(),
                },
                current_value.unwrap_or("<unavailable>").to_owned(),
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
            current: current_value.unwrap_or("<unset>").to_owned(),
            desired: desired_value,
            high_impact: false,
            reference_only: false,
            reason: None,
        });
    }
}

pub(super) fn plan_high_impact_bool_change(
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

pub(super) fn plan_high_impact_string_change(
    changes: &mut Vec<GeneralChange>,
    blocked_changes: &mut Vec<GeneralChange>,
    rest_patch: &mut Map<String, Value>,
    field: &str,
    current_value: Option<&str>,
    desired_value: Option<String>,
    allow_high_impact: bool,
) {
    if let Some(desired_value) = desired_value
        && current_value != Some(desired_value.as_str())
    {
        if allow_high_impact {
            rest_patch.insert(field.to_owned(), json!(desired_value));
            changes.push(GeneralChange {
                kind: GeneralChangeKind::RestField {
                    field: field.to_owned(),
                },
                current: display_optional_string(current_value),
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
                display_optional_string(current_value),
                display_optional_string(Some(desired_value.as_str())),
                "High-impact repository changes require allow_high_impact or a sensitive policy opt-in",
                true,
            ));
        }
    }
}

pub(super) fn blocked_change(
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

pub(super) fn display_optional_string(value: Option<&str>) -> String {
    match value {
        Some("") => "<empty>".to_owned(),
        Some(value) => value.to_owned(),
        None => "<unset>".to_owned(),
    }
}

pub(super) fn display_json_value(value: &Value) -> String {
    match value {
        Value::Null => "<unset>".to_owned(),
        Value::String(value) if value.is_empty() => "<empty>".to_owned(),
        Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<invalid json>".to_owned()),
    }
}

pub(super) fn insert_value(map: &mut Map<String, Value>, field: &str, value: Value) {
    map.insert(field.to_owned(), value);
}

pub(super) fn normalize_optional_policy(value: Option<&str>) -> Option<String> {
    value.map(|value| value.trim().replace(['-', ' '], "_").to_ascii_lowercase())
}
