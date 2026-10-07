//! Ruleset state collection.

use std::collections::HashMap;

use anyhow::Result;

use crate::config::manifest::{
    ActorReference, CategoryPolicy, ManifestCategoryName, RepositoryRuleConfig, RepositoryRuleset,
    RulesetBypassActor, RulesetReferenceConfig, RulesetsCategory,
};
use crate::github::Client;
use crate::github::rulesets::RulesetDetail;
use crate::reconcile::common::actors::{actor_reference_key, repository_role_lookup};
use crate::reconcile::common::coverage::{
    collected_entry, lookup_failure_entry, unavailable_entry,
};
use crate::reconcile::common::rules_issue::ReconcileIssue;
use crate::reconcile::common::rules_issue::blocker_issue;

use super::*;

pub async fn collect_rulesets_category(
    client: &Client,
    repo: &str,
    category: Option<&RulesetsCategory>,
) -> Result<RulesetsCollection> {
    let mut issues = Vec::new();
    let mut coverage = vec![collected_entry(
        ManifestCategoryName::Rulesets,
        "GET /repos/{owner}/{repo}/rulesets",
    )];

    let needs = |matches: fn(&ActorReference) -> bool| {
        category.is_none_or(|category| {
            category
                .repository_rulesets
                .iter()
                .flat_map(|ruleset| &ruleset.bypass_actors)
                .any(|bypass| matches(&bypass.actor))
        })
    };
    let needs_teams = needs(|actor| matches!(actor, ActorReference::Team { .. }));
    let needs_roles = needs(|actor| matches!(actor, ActorReference::Role { .. }));
    let needs_apps = needs(|actor| matches!(actor, ActorReference::App { .. }));
    let needs_users = needs(|actor| matches!(actor, ActorReference::User { .. }));

    let all_rulesets = client.list_rulesets(repo).await?;
    let org_teams = match client.list_org_teams().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/teams",
            ));
            value
        }
        Err(error) => {
            coverage.push(lookup_failure_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/teams",
                format!("{error:#}"),
                needs_teams,
            ));
            Vec::new()
        }
    };
    let custom_roles = match client.list_ruleset_custom_repository_roles().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/custom-repository-roles",
            ));
            value
        }
        Err(error) => {
            coverage.push(lookup_failure_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/custom-repository-roles",
                format!("{error:#}"),
                needs_roles,
            ));
            Vec::new()
        }
    };
    let installed_apps = match client.list_org_installations().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/installations",
            ));
            value
        }
        Err(error) => {
            coverage.push(lookup_failure_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/installations",
                format!("{error:#}"),
                needs_apps,
            ));
            Vec::new()
        }
    };

    let team_by_id: HashMap<u64, String> = org_teams
        .into_iter()
        .map(|team| (team.id, team.slug))
        .collect();
    let app_by_id: HashMap<u64, String> = installed_apps
        .into_iter()
        .map(|app| (app.app_id, app.app_slug))
        .collect();
    let role_by_id = repository_role_lookup(&custom_roles);
    let user_by_id: HashMap<u64, String> = match client.list_ruleset_repo_collaborators(repo).await
    {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /repos/{owner}/{repo}/collaborators?affiliation=all",
            ));
            value
                .into_iter()
                .map(|user| (user.id, user.login))
                .collect()
        }
        Err(error) => {
            coverage.push(lookup_failure_entry(
                ManifestCategoryName::Rulesets,
                "GET /repos/{owner}/{repo}/collaborators?affiliation=all",
                format!("{error:#}"),
                needs_users,
            ));
            HashMap::new()
        }
    };

    let mut actual_repository_rulesets = Vec::new();
    let mut inherited_rulesets = Vec::new();

    for ruleset in all_rulesets {
        if !ruleset.source_type.is_empty() && ruleset.source_type != "Repository" {
            inherited_rulesets.push(RulesetReference {
                name: ruleset.name,
                target: ruleset.target,
                enforcement: ruleset.enforcement,
                source_type: ruleset.source_type,
                source: ruleset.source,
            });
            continue;
        }

        let detail = client
            .get_ruleset(repo, ruleset.id)
            .await
            .map_err(|error| {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Rulesets,
                    "GET /repos/{owner}/{repo}/rulesets/{ruleset_id}",
                    format!("{error:#}"),
                ));
                issues.push(blocker_issue(
                    Some(ruleset.name.clone()),
                    "rulesets-detail-unavailable",
                    format!(
                        "Could not read full ruleset detail for {}: {error}. Planning updates from summary data would be unsafe.",
                        ruleset.name
                    ),
                ));
                error
            })
            .unwrap_or(RulesetDetail {
                id: ruleset.id,
                name: ruleset.name.clone(),
                enforcement: ruleset.enforcement.clone(),
                target: ruleset.target.clone(),
                rules: ruleset.rules.clone(),
                conditions: ruleset.conditions.clone(),
                bypass_actors: ruleset.bypass_actors.clone(),
            });
        let resolved = collect_repository_ruleset(
            detail,
            &team_by_id,
            &app_by_id,
            &role_by_id,
            &user_by_id,
            &ruleset.name,
            &mut issues,
        )?;
        actual_repository_rulesets.push(ActualRepositoryRuleset {
            id: ruleset.id,
            source_type: ruleset.source_type,
            source: ruleset.source,
            ruleset: resolved,
        });
    }

    actual_repository_rulesets.sort_by(|left, right| left.ruleset.name.cmp(&right.ruleset.name));
    inherited_rulesets.sort_by(|left, right| left.name.cmp(&right.name));

    let policy = category
        .map(|value| value.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    Ok(RulesetsCollection {
        category: RulesetsCategory {
            policy,
            references: inherited_rulesets
                .iter()
                .map(|reference| RulesetReferenceConfig {
                    name: reference.name.clone(),
                    target: reference.target.clone(),
                    enforcement: reference.enforcement.clone(),
                    source_type: reference.source_type.clone(),
                    source: reference.source.clone(),
                })
                .collect(),
            repository_rulesets: actual_repository_rulesets
                .iter()
                .map(|ruleset| ruleset.ruleset.clone())
                .collect(),
        },
        actual_repository_rulesets,
        inherited_rulesets,
        team_ids_by_slug: team_by_id
            .iter()
            .map(|(id, slug)| (slug.clone(), *id))
            .collect(),
        repository_role_ids_by_name: role_by_id
            .iter()
            .map(|(id, name)| (name.clone(), *id))
            .collect(),
        app_ids_by_slug: app_by_id
            .iter()
            .map(|(id, slug)| (slug.clone(), *id))
            .collect(),
        coverage,
        issues,
    })
}

fn collect_repository_ruleset(
    detail: RulesetDetail,
    team_by_id: &HashMap<u64, String>,
    app_by_id: &HashMap<u64, String>,
    role_by_id: &HashMap<u64, String>,
    user_by_id: &HashMap<u64, String>,
    resource_name: &str,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<RepositoryRuleset> {
    let conditions_json = detail
        .conditions
        .filter(|value| !value.is_null())
        .map(|value| serde_json::to_string(&value))
        .transpose()?;

    let rules = detail
        .rules
        .into_iter()
        .map(|rule| {
            Ok(RepositoryRuleConfig {
                rule_type: rule.rule_type,
                parameters_json: rule
                    .parameters
                    .map(|parameters| serde_json::to_string(&parameters))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let bypass_actors = detail
        .bypass_actors
        .into_iter()
        .filter_map(|actor| {
            let actor_type = actor.get("actor_type")?.as_str()?.to_owned();
            let actor_id = actor.get("actor_id").and_then(serde_json::Value::as_u64);
            let bypass_mode = actor
                .get("bypass_mode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("always")
                .to_owned();
            let actor = match actor_type.as_str() {
                "OrganizationAdmin" => ActorReference::OrganizationAdmin,
                "Team" => actor
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .map(|slug| ActorReference::Team {
                        slug: slug.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| team_by_id.get(&id))
                            .map(|slug| ActorReference::Team { slug: slug.clone() })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "User" => actor
                    .get("login")
                    .and_then(serde_json::Value::as_str)
                    .map(|login| ActorReference::User {
                        login: login.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| user_by_id.get(&id))
                            .map(|login| ActorReference::User {
                                login: login.clone(),
                            })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "Integration" => actor
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .map(|slug| ActorReference::App {
                        slug: slug.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| app_by_id.get(&id))
                            .map(|slug| ActorReference::App { slug: slug.clone() })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "RepositoryRole" => actor
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(|name| ActorReference::Role {
                        name: name.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| role_by_id.get(&id))
                            .map(|name| ActorReference::Role { name: name.clone() })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "DeployKey" if actor_id.is_none() => ActorReference::Unresolved {
                    actor_type,
                    actor_id: None,
                },
                _ => ActorReference::Unresolved {
                    actor_type,
                    actor_id,
                },
            };
            let is_stable_deploy_key = matches!(
                &actor,
                ActorReference::Unresolved {
                    actor_type,
                    actor_id: None,
                } if actor_type == "DeployKey"
            );
            if matches!(actor, ActorReference::Unresolved { .. }) && !is_stable_deploy_key {
                issues.push(blocker_issue(
                    Some(resource_name.to_owned()),
                    "rulesets-unresolved-bypass-actor",
                    format!(
                        "Ruleset {} contains a bypass actor that could not be resolved to a stable manifest identity: {}",
                        resource_name,
                        actor_reference_key(&actor)
                    ),
                ));
            }
            Some(RulesetBypassActor { actor, bypass_mode })
        })
        .collect::<Vec<_>>();

    Ok(RepositoryRuleset {
        name: detail.name,
        target: if detail.target.is_empty() {
            "branch".to_owned()
        } else {
            detail.target
        },
        enforcement: detail.enforcement,
        conditions_json,
        rules,
        bypass_actors,
    })
}
