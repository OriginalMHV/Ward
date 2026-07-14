use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    ActorReference, CategoryPolicy, ManagementDisposition, RulesetsCategoryV2,
};
use ward::github::Client;
use ward::reconcile::security_rules::{
    RulesetPlanAction, collect_rulesets_category, plan_rulesets_category,
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
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

    let desired = RulesetsCategoryV2 {
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
