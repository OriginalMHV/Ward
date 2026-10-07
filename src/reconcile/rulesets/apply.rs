//! Ruleset apply and verify.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::config::manifest::{
    ActorReference, RepositoryRuleset, RulesetBypassActor, RulesetsCategory,
};
use crate::github::Client;
use crate::reconcile::common::actors::repository_role_lookup;
use crate::reconcile::common::rules_issue::ReconcileIssueSeverity;

use super::collect::*;
use super::plan::*;
use super::*;

pub async fn apply_rulesets_plan(
    client: &Client,
    repo: &str,
    plan: &RulesetsPlan,
) -> Result<RulesetsApplyResult> {
    if let Some(issue) = plan
        .issues
        .iter()
        .find(|issue| issue.severity == ReconcileIssueSeverity::Blocker)
    {
        anyhow::bail!("Rulesets plan is blocked: {}", issue.message);
    }

    let mut applied_steps = Vec::new();
    let requires_role_lookup = plan.actions.iter().any(|action| {
        matches!(
            action,
            RulesetPlanAction::Create { ruleset } | RulesetPlanAction::Update { ruleset, .. }
                if ruleset
                    .bypass_actors
                    .iter()
                    .any(|actor| matches!(&actor.actor, ActorReference::Role { .. }))
        )
    });
    let requires_app_lookup = plan.actions.iter().any(|action| {
        matches!(
            action,
            RulesetPlanAction::Create { ruleset } | RulesetPlanAction::Update { ruleset, .. }
                if ruleset
                    .bypass_actors
                    .iter()
                    .any(|actor| matches!(&actor.actor, ActorReference::App { .. }))
        )
    });
    let role_lookup = if requires_role_lookup {
        repository_role_lookup(
            &client
                .list_ruleset_custom_repository_roles()
                .await
                .context(
                    "Failed to resolve repository roles required by ruleset create/update actions",
                )?,
        )
    } else {
        repository_role_lookup(&[])
    };
    let app_lookup: HashMap<String, u64> = if requires_app_lookup {
        client
            .list_org_installations()
            .await
            .context("Failed to resolve installed apps required by ruleset create/update actions")?
            .into_iter()
            .map(|app| (app.app_slug, app.app_id))
            .collect()
    } else {
        HashMap::new()
    };

    for action in &plan.actions {
        match action {
            RulesetPlanAction::Create { ruleset } => {
                let body =
                    repository_ruleset_to_api_json(client, ruleset, &role_lookup, &app_lookup)
                        .await?;
                client.create_ruleset(repo, &body).await?;
                applied_steps.push(format!("create:{}", ruleset.name));
            }
            RulesetPlanAction::Update {
                ruleset_id,
                ruleset,
            } => {
                let body =
                    repository_ruleset_to_api_json(client, ruleset, &role_lookup, &app_lookup)
                        .await?;
                client.update_ruleset(repo, *ruleset_id, &body).await?;
                applied_steps.push(format!("update:{}", ruleset.name));
            }
            RulesetPlanAction::Delete { ruleset_id, name } => {
                client.delete_ruleset(repo, *ruleset_id).await?;
                applied_steps.push(format!("delete:{name}"));
            }
            RulesetPlanAction::Unchanged { .. } => {}
        }
    }

    Ok(RulesetsApplyResult { applied_steps })
}

pub async fn verify_rulesets_category(
    client: &Client,
    repo: &str,
    desired: &RulesetsCategory,
) -> Result<RulesetsVerifyResult> {
    let actual = collect_rulesets_category(client, repo, Some(desired)).await?;
    let plan = plan_rulesets_category(desired, &actual)?;
    let blocked = plan
        .issues
        .iter()
        .any(|issue| issue.severity == ReconcileIssueSeverity::Blocker);
    Ok(RulesetsVerifyResult {
        matches: !plan.has_changes() && !blocked,
        plan,
    })
}

async fn repository_ruleset_to_api_json(
    client: &Client,
    ruleset: &RepositoryRuleset,
    role_lookup: &HashMap<u64, String>,
    app_lookup: &HashMap<String, u64>,
) -> Result<serde_json::Value> {
    let conditions = ruleset
        .conditions_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .with_context(|| format!("Invalid conditions_json for ruleset {}", ruleset.name))?
        .unwrap_or(serde_json::Value::Null);
    let rules = ruleset
        .rules
        .iter()
        .map(|rule| {
            let mut body = serde_json::json!({ "type": rule.rule_type });
            if let Some(parameters) = rule.parameters_json.as_deref() {
                body["parameters"] = serde_json::from_str(parameters).with_context(|| {
                    format!(
                        "Invalid parameters_json for {} rule in ruleset {}",
                        rule.rule_type, ruleset.name
                    )
                })?;
            }
            Ok(body)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut bypass_actors = Vec::new();
    for actor in &ruleset.bypass_actors {
        bypass_actors
            .push(resolve_ruleset_bypass_actor(client, actor, role_lookup, app_lookup).await?);
    }

    Ok(serde_json::json!({
        "name": ruleset.name,
        "target": ruleset.target,
        "enforcement": ruleset.enforcement,
        "conditions": conditions,
        "rules": rules,
        "bypass_actors": bypass_actors,
    }))
}

async fn resolve_ruleset_bypass_actor(
    client: &Client,
    actor: &RulesetBypassActor,
    role_lookup: &HashMap<u64, String>,
    app_lookup: &HashMap<String, u64>,
) -> Result<serde_json::Value> {
    let (actor_id, actor_type) = match &actor.actor {
        ActorReference::OrganizationAdmin => (None, "OrganizationAdmin".to_owned()),
        ActorReference::Team { slug } => (Some(client.get_team_id(slug).await?), "Team".to_owned()),
        ActorReference::User { login } => (
            Some(client.get_user_by_login(login).await?.id),
            "User".to_owned(),
        ),
        ActorReference::App { slug } => (
            Some(*app_lookup.get(slug).with_context(|| {
                format!(
                    "Installed app {slug} was not found in organization {}",
                    client.org()
                )
            })?),
            "Integration".to_owned(),
        ),
        ActorReference::Role { name } => (
            Some(resolve_repository_role_id(name, role_lookup)?),
            "RepositoryRole".to_owned(),
        ),
        ActorReference::Unresolved {
            actor_type,
            actor_id: _,
        } if actor_type == "DeployKey" => (None, "DeployKey".to_owned()),
        ActorReference::Unresolved { actor_type, .. } => anyhow::bail!(
            "Ruleset bypass actor {} could not be resolved to a supported, stable manifest identity",
            actor_type
        ),
    };

    Ok(serde_json::json!({
        "actor_id": actor_id,
        "actor_type": actor_type,
        "bypass_mode": actor.bypass_mode,
    }))
}

fn resolve_repository_role_id(name: &str, role_lookup: &HashMap<u64, String>) -> Result<u64> {
    role_lookup
        .iter()
        .find_map(|(id, current_name)| current_name.eq_ignore_ascii_case(name).then_some(*id))
        .with_context(|| format!("Repository role {name} is not known in the current organization"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn resolve_ruleset_bypass_actor_preserves_user_deploy_key_org_admin_and_base_role() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/alice"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": 2,
                "login": "alice"
            })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let role_lookup = repository_role_lookup(&[]);
        let app_lookup = HashMap::new();

        let user = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActor {
                actor: ActorReference::User {
                    login: "alice".to_owned(),
                },
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            user,
            json!({
                "actor_id": 2,
                "actor_type": "User",
                "bypass_mode": "always"
            })
        );

        let org_admin = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActor {
                actor: ActorReference::OrganizationAdmin,
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            org_admin,
            json!({
                "actor_id": null,
                "actor_type": "OrganizationAdmin",
                "bypass_mode": "always"
            })
        );

        let base_role = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActor {
                actor: ActorReference::Role {
                    name: "admin".to_owned(),
                },
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            base_role,
            json!({
                "actor_id": 5,
                "actor_type": "RepositoryRole",
                "bypass_mode": "always"
            })
        );

        let deploy_key = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActor {
                actor: ActorReference::Unresolved {
                    actor_type: "DeployKey".to_owned(),
                    actor_id: None,
                },
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            deploy_key,
            json!({
                "actor_id": null,
                "actor_type": "DeployKey",
                "bypass_mode": "always"
            })
        );
    }
}
