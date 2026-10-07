//! Topic, custom property, immutable release and label planning and normalisation.

use super::plan::{blocked_change, desired_setting_value, display_json_value, repository_to_value};
use super::{
    CollectedGeneralState, GeneralChange, GeneralChangeKind, GeneralCustomPropertyValue,
    GeneralDesiredState, GeneralLabel, GeneralResourceAction, PlannedCustomPropertyUpdate,
    PlannedImmutableReleaseAction, PlannedLabelAction,
};
use crate::config::manifest::{
    CustomPropertyValueConfig, ImmutableReleasesConfig, RepositoryCategory,
};
use crate::github::settings::{RepositoryCustomPropertyValue, RepositoryLabel};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn collect_custom_properties(
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

pub(super) fn collect_labels(labels: Vec<RepositoryLabel>) -> Vec<GeneralLabel> {
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

pub(super) fn manifest_compatible_custom_properties(
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

pub(super) fn planned_topics(
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

pub(super) fn planned_custom_property_updates(
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

pub(super) fn planned_immutable_release_action(
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

pub(super) fn planned_label_actions(
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

pub(super) fn label_needs_update(current: &GeneralLabel, desired: &GeneralLabel) -> bool {
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

pub(super) fn desired_topics(desired: &GeneralDesiredState) -> Option<Vec<String>> {
    let value = desired_setting_value(desired, "topics")?;
    let values = value
        .as_array()?
        .iter()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    Some(normalize_topics(&values))
}

pub(super) fn current_custom_properties(
    current: &CollectedGeneralState,
) -> Vec<GeneralCustomPropertyValue> {
    if !current.custom_properties.is_empty() {
        return current.custom_properties.clone();
    }
    extract_manifest_custom_properties(&current.repository)
}

pub(super) fn desired_custom_properties(
    desired: &GeneralDesiredState,
) -> Vec<GeneralCustomPropertyValue> {
    if !desired.custom_properties.is_empty() {
        return desired.custom_properties.clone();
    }
    extract_manifest_custom_properties(&desired.repository)
}

pub(super) fn extract_manifest_custom_properties(
    repository: &RepositoryCategory,
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

pub(super) fn normalize_label_color(value: &str) -> String {
    value.trim_start_matches('#').to_ascii_lowercase()
}

pub(super) fn normalize_topics(topics: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for topic in topics {
        let topic = topic.trim().to_lowercase();
        if !topic.is_empty() && seen.insert(topic.clone()) {
            normalized.push(topic);
        }
    }
    normalized
}

pub(super) fn union_topics(current: &[String], desired: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut topics = Vec::new();
    for topic in current.iter().chain(desired.iter()) {
        if seen.insert(topic.clone()) {
            topics.push(topic.clone());
        }
    }
    topics
}

pub(super) fn topic_set(topics: &[String]) -> BTreeSet<String> {
    topics.iter().cloned().collect()
}

pub(super) fn display_immutable_releases(value: &ImmutableReleasesConfig) -> String {
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

pub(super) fn format_topics(topics: &[String]) -> String {
    format!("{topics:?}")
}
