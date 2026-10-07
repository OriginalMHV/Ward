//! Ruleset plan computation.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::config::manifest::{
    ActorReference, ManagementDisposition, RepositoryRuleConfig, RepositoryRuleset,
    RulesetBypassActor, RulesetReferenceConfig, RulesetsCategory,
};
use crate::reconcile::common::actors::actor_reference_key;
use crate::reconcile::common::rules_issue::ReconcileIssue;
use crate::reconcile::common::rules_issue::{blocker_issue, warning_issue};

use super::*;

pub fn plan_rulesets_category(
    desired: &RulesetsCategory,
    actual: &RulesetsCollection,
) -> Result<RulesetsPlan> {
    let mut issues = actual.issues.clone();
    let mut actions = Vec::new();

    let actual_by_name: BTreeMap<&str, &ActualRepositoryRuleset> = actual
        .actual_repository_rulesets
        .iter()
        .map(|ruleset| (ruleset.ruleset.name.as_str(), ruleset))
        .collect();
    let desired_names: BTreeSet<&str> = desired
        .repository_rulesets
        .iter()
        .map(|ruleset| ruleset.name.as_str())
        .collect();

    if desired.policy.disposition != ManagementDisposition::Managed {
        for ruleset in &actual.actual_repository_rulesets {
            actions.push(RulesetPlanAction::Unchanged {
                name: ruleset.ruleset.name.clone(),
            });
        }
        return Ok(RulesetsPlan { actions, issues });
    }

    if normalize_ruleset_references(&desired.references)
        != normalize_ruleset_references(&actual.category.references)
    {
        issues.push(blocker_issue(
            Some("categories.rulesets.references".to_owned()),
            "rulesets-inherited-reference-drift",
            "Inherited ruleset references differ from the live repository state and are reference-only at repository scope.".to_owned(),
        ));
    }

    for desired_ruleset in &desired.repository_rulesets {
        validate_ruleset_bypass_actors(desired_ruleset, &mut issues);
        match actual_by_name.get(desired_ruleset.name.as_str()) {
            None => actions.push(RulesetPlanAction::Create {
                ruleset: desired_ruleset.clone(),
            }),
            Some(existing) => {
                if repository_ruleset_matches(desired_ruleset, &existing.ruleset) {
                    actions.push(RulesetPlanAction::Unchanged {
                        name: desired_ruleset.name.clone(),
                    });
                } else {
                    actions.push(RulesetPlanAction::Update {
                        ruleset_id: existing.id,
                        ruleset: desired_ruleset.clone(),
                    });
                }
            }
        }
    }

    if desired.policy.prune {
        for existing in &actual.actual_repository_rulesets {
            if !desired_names.contains(existing.ruleset.name.as_str()) {
                actions.push(RulesetPlanAction::Delete {
                    ruleset_id: existing.id,
                    name: existing.ruleset.name.clone(),
                });
            }
        }
    }

    actions.sort_by_key(ruleset_action_sort_key);

    if !actual.inherited_rulesets.is_empty() {
        issues.push(warning_issue(
            Some("categories.rulesets.references".to_owned()),
            "rulesets-inherited-reference-only",
            "Inherited rulesets are recorded as references only and will never be mutated by repository reconciliation.".to_owned(),
        ));
    }

    if actions
        .iter()
        .any(|action| !matches!(action, RulesetPlanAction::Unchanged { .. }))
        && !desired.policy.sensitive
    {
        issues.push(blocker_issue(
            Some("categories.rulesets.policy.sensitive".to_owned()),
            "rulesets-sensitive-gate",
            "Managing repository rulesets requires policy.sensitive = true".to_owned(),
        ));
    }

    Ok(RulesetsPlan { actions, issues })
}

fn repository_ruleset_matches(left: &RepositoryRuleset, right: &RepositoryRuleset) -> bool {
    left.name == right.name
        && left.target == right.target
        && left.enforcement == right.enforcement
        && normalized_json_string(left.conditions_json.as_deref())
            == normalized_json_string(right.conditions_json.as_deref())
        && normalized_rules(left.rules.as_slice()) == normalized_rules(right.rules.as_slice())
        && normalized_bypass_actors(left.bypass_actors.as_slice())
            == normalized_bypass_actors(right.bypass_actors.as_slice())
}

fn normalized_json_string(value: Option<&str>) -> Option<String> {
    value
        .map(|value| {
            serde_json::from_str::<serde_json::Value>(value)
                .unwrap_or(serde_json::Value::String(value.to_owned()))
        })
        .and_then(|value| serde_json::to_string(&value).ok())
}

fn normalized_rules(rules: &[RepositoryRuleConfig]) -> Vec<(String, Option<String>)> {
    let mut normalized = rules
        .iter()
        .map(|rule| {
            (
                rule.rule_type.clone(),
                normalized_json_string(rule.parameters_json.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalized_bypass_actors(actors: &[RulesetBypassActor]) -> Vec<(String, String)> {
    let mut normalized = actors
        .iter()
        .map(|actor| (actor_reference_key(&actor.actor), actor.bypass_mode.clone()))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_ruleset_references(
    references: &[RulesetReferenceConfig],
) -> Vec<(String, String, String, String, String)> {
    let mut normalized = references
        .iter()
        .map(|reference| {
            (
                reference.name.clone(),
                reference.target.clone(),
                reference.enforcement.clone(),
                reference.source_type.clone(),
                reference.source.clone(),
            )
        })
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn validate_ruleset_bypass_actors(ruleset: &RepositoryRuleset, issues: &mut Vec<ReconcileIssue>) {
    for actor in &ruleset.bypass_actors {
        match &actor.actor {
            ActorReference::Unresolved {
                actor_type,
                actor_id,
            } if actor_type == "DeployKey" && actor_id.is_none() => {}
            ActorReference::Unresolved {
                actor_type,
                actor_id,
            } => {
                issues.push(blocker_issue(
                    Some(ruleset.name.clone()),
                    "rulesets-unresolved-bypass-actor",
                    format!(
                        "Ruleset {} declares unresolved bypass actor {}{} and cannot be safely applied.",
                        ruleset.name,
                        actor_type,
                        actor_id
                            .map(|id| format!(":{id}"))
                            .unwrap_or_default()
                    ),
                ));
            }
            _ => {}
        }
    }
}

fn ruleset_action_sort_key(action: &RulesetPlanAction) -> (u8, String) {
    match action {
        RulesetPlanAction::Create { ruleset } => (0, ruleset.name.clone()),
        RulesetPlanAction::Update { ruleset, .. } => (1, ruleset.name.clone()),
        RulesetPlanAction::Delete { name, .. } => (2, name.clone()),
        RulesetPlanAction::Unchanged { name } => (3, name.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruleset_match_is_order_insensitive() {
        let left = RepositoryRuleset {
            name: "main".to_owned(),
            target: "branch".to_owned(),
            enforcement: "active".to_owned(),
            conditions_json: Some(
                r#"{"ref_name":{"include":["~DEFAULT_BRANCH"],"exclude":[]}}"#.to_owned(),
            ),
            rules: vec![
                RepositoryRuleConfig {
                    rule_type: "deletion".to_owned(),
                    parameters_json: None,
                },
                RepositoryRuleConfig {
                    rule_type: "required_signatures".to_owned(),
                    parameters_json: None,
                },
            ],
            bypass_actors: vec![RulesetBypassActor {
                actor: ActorReference::Team {
                    slug: "platform".to_owned(),
                },
                bypass_mode: "always".to_owned(),
            }],
        };
        let mut right = left.clone();
        right.rules.reverse();

        assert!(repository_ruleset_matches(&left, &right));
    }
}
