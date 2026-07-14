//! Reconcile-layer tests for the Environments category:
//! `collect_environments_category` / `plan_environments_category` /
//! `apply_environments_plan` / `verify_environments_category`.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    ActorReference, CategoryPolicy, CoverageOutcome, EnvironmentConfigV2,
    EnvironmentDeploymentPolicyConfig, EnvironmentReviewerConfig, EnvironmentsCategoryV2,
    ExternalValueReference, ManagementDisposition, ManifestCategoryName, NamedValueConfig,
    ReferencedResourceConfig, ReferencedResourceType, SecretPlaceholderConfig,
};
use ward::github::Client;
use ward::reconcile::actions_environments::{
    IssueSeverity, apply_environments_plan, collect_environments_category,
    plan_environments_category, verify_environments_category,
};

fn client(server: &MockServer) -> Client {
    Client::new_for_test("test-org", &server.uri())
}

fn managed_policy(prune: bool) -> CategoryPolicy {
    CategoryPolicy {
        disposition: ManagementDisposition::Managed,
        prune,
        sensitive: false,
    }
}

/// Mount empty (no branch policies / protection apps / variables / secrets)
/// per-environment sub-resource responses so `collect_environments_category`
/// can complete without additional per-test boilerplate for scenarios that
/// don't exercise those sub-resources.
async fn mount_empty_environment_subresources(server: &MockServer, repo: &str, env: &str) {
    for suffix in [
        "deployment-branch-policies",
        "deployment_protection_rules",
        "variables",
        "secrets",
    ] {
        let body = match suffix {
            "deployment-branch-policies" => json!({"branch_policies": []}),
            "deployment_protection_rules" => json!({"custom_deployment_protection_rules": []}),
            "variables" => json!({"variables": []}),
            _ => json!({"secrets": []}),
        };
        Mock::given(method("GET"))
            .and(path(format!(
                "/repos/test-org/{repo}/environments/{env}/{suffix}"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn collect_lists_environment_settings_and_reviewers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{
                "name": "production",
                "deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true},
                "protection_rules": [
                    {"id": 1, "type": "wait_timer", "wait_timer": 10},
                    {
                        "id": 2,
                        "type": "required_reviewers",
                        "prevent_self_review": true,
                        "reviewers": [
                            {"type": "User", "reviewer": {"id": 1, "login": "alice"}},
                            {"type": "Team", "reviewer": {"id": 2, "slug": "release-approvers"}}
                        ]
                    }
                ]
            }]
        })))
        .mount(&server)
        .await;
    mount_empty_environment_subresources(&server, "my-repo", "production").await;

    let collected = collect_environments_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    assert_eq!(collected.category.entries.len(), 1);
    let env = &collected.category.entries[0];
    assert_eq!(env.name, "production");
    assert_eq!(env.wait_timer_minutes, Some(10));
    assert_eq!(env.prevent_self_review, Some(true));
    assert_eq!(env.reviewers.len(), 2);
    assert!(env.reviewers.iter().any(|r| r.actor
        == ActorReference::User {
            login: "alice".to_owned()
        }));
    assert!(env.reviewers.iter().any(|r| r.actor
        == ActorReference::Team {
            slug: "release-approvers".to_owned()
        }));
}

#[tokio::test]
async fn plan_creates_missing_environment() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "staging".to_owned(),
            wait_timer_minutes: Some(5),
            ..Default::default()
        }],
    };

    let collected = collect_environments_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);

    assert_eq!(plan.environment_plans.len(), 1);
    assert!(plan.environment_plans[0].create);
    assert!(plan.environment_plans[0].settings_change.is_some());
}

#[tokio::test]
async fn plan_is_idempotent_when_settings_already_match() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{
                "name": "staging",
                "protection_rules": [
                    {"id": 1, "type": "wait_timer", "wait_timer": 5},
                    {"id": 2, "type": "required_reviewers", "prevent_self_review": false, "reviewers": []}
                ]
            }]
        })))
        .mount(&server)
        .await;
    mount_empty_environment_subresources(&server, "my-repo", "staging").await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "staging".to_owned(),
            wait_timer_minutes: Some(5),
            prevent_self_review: Some(false),
            ..Default::default()
        }],
    };

    let collected = collect_environments_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);

    assert!(
        !plan.environment_plans[0].has_actionable_changes(),
        "expected no-op, got: {:?}",
        plan.environment_plans[0]
    );
}

#[tokio::test]
async fn apply_resolves_reviewer_actors_to_ids_before_put() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": 101, "login": "alice"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/teams/release-approvers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 202})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/environments/production"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "production",
            "protection_rules": []
        })))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            wait_timer_minutes: Some(15),
            reviewers: vec![
                EnvironmentReviewerConfig {
                    actor: ActorReference::User {
                        login: "alice".to_owned(),
                    },
                },
                EnvironmentReviewerConfig {
                    actor: ActorReference::Team {
                        slug: "release-approvers".to_owned(),
                    },
                },
            ],
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();

    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );
    assert!(
        result
            .applied
            .iter()
            .any(|scope| scope == "environments.production")
    );
}

#[tokio::test]
async fn apply_blocks_when_reviewer_actor_cannot_be_resolved() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/ghost"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            reviewers: vec![EnvironmentReviewerConfig {
                actor: ActorReference::User {
                    login: "ghost".to_owned(),
                },
            }],
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();

    assert!(
        result
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker
                && issue.scope.contains("reviewers"))
    );
    // The blocked reviewer resolution must prevent the PUT from being sent at all.
    assert!(
        !result
            .applied
            .iter()
            .any(|scope| scope == "environments.production")
    );
}

#[tokio::test]
async fn apply_blocks_when_actor_kind_is_unsupported_for_reviewers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            reviewers: vec![EnvironmentReviewerConfig {
                actor: ActorReference::App {
                    slug: "some-bot".to_owned(),
                },
            }],
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();

    assert!(
        result
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker)
    );
}

#[tokio::test]
async fn deployment_branch_and_tag_patterns_are_created_when_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{
                "name": "production",
                "deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true},
                "reviewers": [],
                "protection_rules": []
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment-branch-policies",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"branch_policies": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"custom_deployment_protection_rules": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/variables",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"variables": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/secrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"secrets": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment-branch-policies",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1,
            "name": "release/*",
            "type": "branch"
        })))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            deployment_policy: Some(EnvironmentDeploymentPolicyConfig {
                protected_branches: Some(false),
                custom_branch_policies: Some(true),
                branch_patterns: vec!["release/*".to_owned()],
                tag_patterns: vec![],
            }),
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    assert_eq!(
        plan.environment_plans[0].branch_policy_creates,
        vec![("release/*".to_owned(), "branch".to_owned())]
    );

    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();
    assert!(result.issues.is_empty());
}

#[tokio::test]
async fn branch_patterns_without_custom_policy_enabled_is_a_warning() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            deployment_policy: Some(EnvironmentDeploymentPolicyConfig {
                protected_branches: Some(false),
                custom_branch_policies: Some(false),
                branch_patterns: vec!["release/*".to_owned()],
                tag_patterns: vec![],
            }),
            ..Default::default()
        }],
    };

    let collected = collect_environments_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);

    assert!(plan.environment_plans[0].branch_policy_creates.is_empty());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Warning)
    );
}

#[tokio::test]
async fn protection_app_is_enabled_when_available() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{"name": "production", "reviewers": [], "protection_rules": []}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment-branch-policies",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"branch_policies": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/variables",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"variables": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/secrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"secrets": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"custom_deployment_protection_rules": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules/apps",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "available_custom_deployment_protection_rule_integrations": [
                {"id": 55, "slug": "security-gate"}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1,
            "enabled": true,
            "app": {"id": 55, "slug": "security-gate"}
        })))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            protection_apps: vec![ReferencedResourceConfig {
                resource_type: ReferencedResourceType::App,
                name: "security-gate".to_owned(),
            }],
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    assert_eq!(
        plan.environment_plans[0].protection_app_enables,
        vec!["security-gate".to_owned()]
    );

    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();
    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );
}

#[tokio::test]
async fn protection_app_unavailable_is_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules/apps",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "available_custom_deployment_protection_rule_integrations": []
        })))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            protection_apps: vec![ReferencedResourceConfig {
                resource_type: ReferencedResourceType::App,
                name: "not-installed".to_owned(),
            }],
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();

    assert!(
        result
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker
                && issue.scope.contains("not-installed"))
    );
}

#[tokio::test]
async fn environment_prune_deletes_environments_not_in_desired() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [
                {"name": "production", "reviewers": [], "protection_rules": []},
                {"name": "legacy-preview", "reviewers": [], "protection_rules": []}
            ]
        })))
        .mount(&server)
        .await;
    mount_empty_environment_subresources(&server, "my-repo", "production").await;
    mount_empty_environment_subresources(&server, "my-repo", "legacy-preview").await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/my-repo/environments/legacy-preview"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(true),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    assert_eq!(
        plan.environment_deletions,
        vec!["legacy-preview".to_owned()]
    );

    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();
    assert!(result.issues.is_empty());
}

#[tokio::test]
async fn environment_variable_and_secret_upserts_and_prune() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{"name": "production", "reviewers": [], "protection_rules": []}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment-branch-policies",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"branch_policies": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"custom_deployment_protection_rules": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/variables",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "variables": [{"name": "STALE_VAR", "value": "x"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/secrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "secrets": [{"name": "STALE_SECRET"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/variables",
        ))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/variables/STALE_VAR",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/secrets/STALE_SECRET",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(true),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            variables: vec![NamedValueConfig {
                name: "NEW_VAR".to_owned(),
                value: "value".to_owned(),
            }],
            secrets: vec![],
            ..Default::default()
        }],
    };

    let client = client(&server);
    let collected = collect_environments_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);
    assert_eq!(plan.environment_plans[0].variable_upserts.len(), 1);
    assert_eq!(
        plan.environment_plans[0].variable_deletions,
        vec!["STALE_VAR".to_owned()]
    );
    assert_eq!(
        plan.environment_plans[0].secret_deletions,
        vec!["STALE_SECRET".to_owned()]
    );

    let result = apply_environments_plan(&client, "my-repo", &plan)
        .await
        .unwrap();
    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );
}

#[tokio::test]
async fn environment_secret_placeholder_with_unresolved_manual_value_is_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            secrets: vec![SecretPlaceholderConfig {
                name: "API_KEY".to_owned(),
                value_from: ExternalValueReference::Manual {
                    hint: Some("provide via vault".to_owned()),
                },
            }],
            ..Default::default()
        }],
    };

    let collected = collect_environments_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);

    assert!(
        plan.issues.iter().any(
            |issue| issue.severity == IssueSeverity::Blocker && issue.scope.contains("API_KEY")
        )
    );
}

#[tokio::test]
async fn verify_environments_reports_compliant_when_matching() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{
                "name": "staging",
                "protection_rules": []
            }]
        })))
        .mount(&server)
        .await;
    mount_empty_environment_subresources(&server, "my-repo", "staging").await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "staging".to_owned(),
            ..Default::default()
        }],
    };

    let result = verify_environments_category(&client(&server), "my-repo", &desired)
        .await
        .unwrap();
    assert!(
        result.compliant,
        "expected compliant, plan: {:?}",
        result.plan
    );
}

// ---------------------------------------------------------------------------
// Hardening: reviewer/prevent_self_review extraction must come from
// `protection_rules`, not a (generally absent in practice) top-level field
// (issue #5).
// ---------------------------------------------------------------------------

/// When there is no `required_reviewers` protection rule at all (only a
/// `wait_timer` rule), reviewers must be empty and `prevent_self_review`
/// must be `None` (absence), not `Some(false)`.
#[tokio::test]
async fn reviewer_extraction_is_absent_without_a_required_reviewers_rule() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{
                "name": "production",
                "protection_rules": [
                    {"id": 1, "type": "wait_timer", "wait_timer": 30}
                ]
            }]
        })))
        .mount(&server)
        .await;
    mount_empty_environment_subresources(&server, "my-repo", "production").await;

    let collected = collect_environments_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    let env = &collected.category.entries[0];
    assert_eq!(env.wait_timer_minutes, Some(30));
    assert_eq!(env.prevent_self_review, None);
    assert!(env.reviewers.is_empty());
}

/// A `required_reviewers` rule with an empty `reviewers` array (but
/// `prevent_self_review` present) must still surface `prevent_self_review`,
/// distinguishing "no rule" from "rule with no reviewers".
#[tokio::test]
async fn reviewer_extraction_surfaces_prevent_self_review_with_no_reviewers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{
                "name": "production",
                "protection_rules": [
                    {"id": 1, "type": "required_reviewers", "prevent_self_review": true, "reviewers": []}
                ]
            }]
        })))
        .mount(&server)
        .await;
    mount_empty_environment_subresources(&server, "my-repo", "production").await;

    let collected = collect_environments_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    let env = &collected.category.entries[0];
    assert_eq!(env.prevent_self_review, Some(true));
    assert!(env.reviewers.is_empty());
}

// ---------------------------------------------------------------------------
// Hardening: classified reads on environment sub-endpoints must not abort
// collection of other environments (issue #1).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn permission_denied_on_one_environments_sub_endpoint_still_collects_rest() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{"name": "production", "protection_rules": []}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment-branch-policies",
        ))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Must have admin rights to Repository."
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"custom_deployment_protection_rules": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/variables",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"variables": [{"name": "REGION", "value": "eu-west-1"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/secrets",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"secrets": []})))
        .mount(&server)
        .await;

    let collected = collect_environments_category(&client(&server), "my-repo", None)
        .await
        .expect("403 on one sub-endpoint must not abort environment collection");

    // The permission-denied sub-resource yields no branch-policy patterns...
    let env = &collected.category.entries[0];
    assert!(
        env.deployment_policy
            .as_ref()
            .map(|policy| policy.branch_patterns.is_empty() && policy.tag_patterns.is_empty())
            .unwrap_or(true)
    );
    // ...but variables (a sibling sub-endpoint) still collected normally.
    assert_eq!(env.variables.len(), 1);
    assert_eq!(env.variables[0].name, "REGION");
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "environments/production/deployment-branch-policies"
            && entry.outcome == CoverageOutcome::PermissionDenied
            && entry.category == ManifestCategoryName::Environments
    }));
}

// ---------------------------------------------------------------------------
// Hardening: environment secret idempotence by name (issue #6).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn already_present_environment_secret_is_not_replanned_and_verify_converges() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "environments": [{"name": "production", "protection_rules": []}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment-branch-policies",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"branch_policies": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment_protection_rules",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"custom_deployment_protection_rules": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/variables",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"variables": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/secrets",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"secrets": [{"name": "API_KEY"}]})),
        )
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(false),
        entries: vec![EnvironmentConfigV2 {
            name: "production".to_owned(),
            secrets: vec![SecretPlaceholderConfig {
                name: "API_KEY".to_owned(),
                // Deliberately unresolvable: no env var set, no hint.
                value_from: ExternalValueReference::Manual { hint: None },
            }],
            ..Default::default()
        }],
    };

    let collected = collect_environments_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_environments_category(&desired, &collected);

    assert!(
        plan.environment_plans[0].secret_upserts.is_empty(),
        "an already-present secret must not be resolved/upserted: {:?}",
        plan.environment_plans[0]
    );
    assert!(
        !plan
            .issues
            .iter()
            .any(|issue| issue.scope.contains("API_KEY")),
        "an already-present secret must not produce an unresolved-value blocker: {:?}",
        plan.issues
    );

    let result = verify_environments_category(&client(&server), "my-repo", &desired)
        .await
        .unwrap();
    assert!(
        result.compliant,
        "verify must converge once the secret already exists: {:?}",
        result.plan
    );
}

// ---------------------------------------------------------------------------
// Collected snapshot policy: an import (`desired = None`) must default to
// `observe_sensitive()`, and a reconcile pass (`desired = Some`) must
// preserve the caller's policy rather than silently resetting it to a
// plain, non-sensitive `Observe` default.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_snapshot_policy_defaults_to_observe_sensitive() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;

    let collected = collect_environments_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    assert_eq!(
        collected.category.policy,
        CategoryPolicy {
            disposition: ManagementDisposition::Observe,
            prune: false,
            sensitive: true,
        },
        "an import snapshot must default to observe_sensitive(), not a plain Observe default"
    );
}

#[tokio::test]
async fn reconcile_snapshot_preserves_desired_policy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": []})))
        .mount(&server)
        .await;

    let desired = EnvironmentsCategoryV2 {
        policy: managed_policy(true),
        ..Default::default()
    };

    let collected = collect_environments_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();

    assert_eq!(
        collected.category.policy,
        managed_policy(true),
        "the collected snapshot must preserve the caller's desired policy, not reset it"
    );
}
