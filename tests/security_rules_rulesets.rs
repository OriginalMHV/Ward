#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    ActorReference, CategoryPolicy, ManagementDisposition, RepositoryRuleset, RulesetBypassActor,
    RulesetsCategory,
};
use ward::github::Client;
use ward::reconcile::security_rules::{
    RulesetPlanAction, RulesetsPlan, apply_rulesets_plan, collect_rulesets_category,
    plan_rulesets_category,
};

#[tokio::test]
async fn security_rules_rulesets_remap_actors_and_prune_only_repository_owned_rulesets() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/rulesets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 10,
                "name": "Owned main",
                "target": "branch",
                "source_type": "Repository",
                "source": "test-org/example",
                "enforcement": "active"
            },
            {
                "id": 11,
                "name": "Org inherited",
                "target": "branch",
                "source_type": "Organization",
                "source": "test-org",
                "enforcement": "active"
            }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 1, "slug": "platform", "name": "Platform" }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/collaborators"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 2, "login": "alice" }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/custom-repository-roles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 0,
            "custom_roles": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 1,
            "installations": [
                { "app_id": 3, "app_slug": "release-bot" }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/rulesets/10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 10,
            "name": "Owned main",
            "target": "branch",
            "enforcement": "active",
            "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
            "rules": [{ "type": "deletion" }],
            "bypass_actors": [
                { "actor_type": "Team", "actor_id": 1, "bypass_mode": "always" },
                { "actor_type": "User", "actor_id": 2, "bypass_mode": "always" },
                { "actor_type": "Integration", "actor_id": 3, "bypass_mode": "always" },
                { "actor_type": "RepositoryRole", "actor_id": 5, "bypass_mode": "always" },
                { "actor_type": "OrganizationAdmin", "actor_id": 0, "bypass_mode": "always" },
                { "actor_type": "DeployKey", "actor_id": null, "bypass_mode": "always" }
            ]
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_rulesets_category(&client, "example", None)
        .await
        .unwrap();

    assert_eq!(collected.actual_repository_rulesets.len(), 1);
    assert_eq!(collected.inherited_rulesets.len(), 1);
    assert_eq!(collected.category.references.len(), 1);
    assert_eq!(collected.category.references[0].name, "Org inherited");
    let actors = &collected.actual_repository_rulesets[0]
        .ruleset
        .bypass_actors;
    assert!(
        actors.iter().any(
            |actor| matches!(&actor.actor, ActorReference::Team { slug } if slug == "platform")
        )
    );
    assert!(
        actors.iter().any(
            |actor| matches!(&actor.actor, ActorReference::User { login } if login == "alice")
        )
    );
    assert!(actors.iter().any(
        |actor| matches!(&actor.actor, ActorReference::App { slug } if slug == "release-bot")
    ));
    assert!(
        actors
            .iter()
            .any(|actor| matches!(&actor.actor, ActorReference::Role { name } if name == "admin"))
    );
    assert!(
        actors
            .iter()
            .any(|actor| matches!(&actor.actor, ActorReference::OrganizationAdmin))
    );
    assert!(actors.iter().any(|actor| matches!(
        &actor.actor,
        ActorReference::Unresolved { actor_type, actor_id: None } if actor_type == "DeployKey"
    )));
    assert!(
        !collected
            .issues
            .iter()
            .any(|issue| issue.code == "rulesets-unsupported-user-bypass-actor")
    );

    let desired = RulesetsCategory {
        policy: CategoryPolicy {
            disposition: ManagementDisposition::Managed,
            prune: true,
            sensitive: false,
        },
        references: Vec::new(),
        repository_rulesets: Vec::new(),
    };
    let plan = plan_rulesets_category(&desired, &collected).unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert!(
        matches!(&plan.actions[0], RulesetPlanAction::Delete { name, .. } if name == "Owned main")
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.code == "rulesets-sensitive-gate")
    );
}

#[tokio::test]
async fn ruleset_apply_skips_actor_lookups_for_delete_only_plan() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/example/rulesets/10"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let plan = RulesetsPlan {
        actions: vec![RulesetPlanAction::Delete {
            ruleset_id: 10,
            name: "obsolete".to_owned(),
        }],
        issues: Vec::new(),
    };

    let result = apply_rulesets_plan(&client, "example", &plan)
        .await
        .unwrap();

    assert_eq!(result.applied_steps, vec!["delete:obsolete"]);
}

#[tokio::test]
async fn ruleset_apply_propagates_required_app_lookup_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/installations"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "message": "installation lookup failed"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let plan = RulesetsPlan {
        actions: vec![RulesetPlanAction::Create {
            ruleset: RepositoryRuleset {
                name: "main".to_owned(),
                target: "branch".to_owned(),
                enforcement: "active".to_owned(),
                conditions_json: None,
                rules: Vec::new(),
                bypass_actors: vec![RulesetBypassActor {
                    actor: ActorReference::App {
                        slug: "release-bot".to_owned(),
                    },
                    bypass_mode: "always".to_owned(),
                }],
            },
        }],
        issues: Vec::new(),
    };

    let error = apply_rulesets_plan(&client, "example", &plan)
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("Failed to resolve installed apps required by ruleset create/update actions")
    );
}

#[tokio::test]
async fn ruleset_apply_propagates_required_role_lookup_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/custom-repository-roles"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "message": "role lookup failed"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let plan = RulesetsPlan {
        actions: vec![RulesetPlanAction::Create {
            ruleset: RepositoryRuleset {
                name: "main".to_owned(),
                target: "branch".to_owned(),
                enforcement: "active".to_owned(),
                conditions_json: None,
                rules: Vec::new(),
                bypass_actors: vec![RulesetBypassActor {
                    actor: ActorReference::Role {
                        name: "release-manager".to_owned(),
                    },
                    bypass_mode: "always".to_owned(),
                }],
            },
        }],
        issues: Vec::new(),
    };

    let error = apply_rulesets_plan(&client, "example", &plan)
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains(
            "Failed to resolve repository roles required by ruleset create/update actions"
        )
    );
}

#[tokio::test]
async fn forbidden_lookups_the_manifest_does_not_need_are_not_unknown_state() {
    use ward::config::manifest::CoverageOutcome;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/rulesets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "Forbidden"})))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let desired = RulesetsCategory {
        policy: CategoryPolicy {
            disposition: ManagementDisposition::Managed,
            prune: false,
            sensitive: true,
        },
        references: Vec::new(),
        repository_rulesets: Vec::new(),
    };
    let collected = collect_rulesets_category(&client, "example", Some(&desired))
        .await
        .unwrap();

    assert!(collected.coverage.iter().all(|entry| !matches!(
        entry.outcome,
        CoverageOutcome::PermissionDenied | CoverageOutcome::Unavailable
    )));
}

#[tokio::test]
async fn plan_limit_403_on_rulesets_names_the_status_and_github_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/rulesets"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Upgrade to GitHub Pro or make this repository public to enable this feature."
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let error = collect_rulesets_category(&client, "example", None)
        .await
        .unwrap_err();
    let message = format!("{error:#}");

    assert!(message.contains("403"), "{message}");
    assert!(message.contains("Upgrade to GitHub Pro"), "{message}");
    assert!(!message.contains("Failed to parse"), "{message}");
}

fn copilot_snippet_ruleset() -> RulesetsCategory {
    let manifest: ward::config::Manifest = toml::from_str(&format!(
        "[org]\nname = \"test-org\"\n\n[schema]\nversion = 2\n\n[categories.rulesets.policy]\ndisposition = \"managed\"\nsensitive = true\n\n{}\n",
        ward::cli::deprecated::COPILOT_REVIEW_SNIPPET
    ))
    .unwrap();
    manifest.categories.rulesets.unwrap()
}

#[tokio::test]
async fn copilot_review_snippet_plans_create_on_an_empty_repository() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/rulesets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let desired = copilot_snippet_ruleset();
    let collected = collect_rulesets_category(&client, "example", Some(&desired))
        .await
        .unwrap();
    let plan = plan_rulesets_category(&desired, &collected).unwrap();

    assert!(
        matches!(plan.actions.as_slice(), [RulesetPlanAction::Create { ruleset }] if ruleset.name == "Copilot Code Review"),
        "{:?}",
        plan.actions
    );
    assert!(plan.issues.is_empty(), "{:?}", plan.issues);
}

#[tokio::test]
async fn copilot_review_snippet_is_a_noop_when_github_echoes_it_back() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/rulesets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 7,
            "name": "Copilot Code Review",
            "target": "branch",
            "source_type": "Repository",
            "source": "test-org/example",
            "enforcement": "active"
        }])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/rulesets/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7,
            "name": "Copilot Code Review",
            "target": "branch",
            "source_type": "Repository",
            "source": "test-org/example",
            "enforcement": "active",
            "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
            "rules": [{
                "type": "copilot_code_review",
                "parameters": { "review_on_push": true, "review_draft_pull_requests": false }
            }],
            "bypass_actors": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let desired = copilot_snippet_ruleset();
    let collected = collect_rulesets_category(&client, "example", Some(&desired))
        .await
        .unwrap();
    let plan = plan_rulesets_category(&desired, &collected).unwrap();

    assert!(
        matches!(plan.actions.as_slice(), [RulesetPlanAction::Unchanged { name }] if name == "Copilot Code Review"),
        "{:?}",
        plan.actions
    );
    assert!(plan.issues.is_empty(), "{:?}", plan.issues);
}
