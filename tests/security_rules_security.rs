use std::collections::HashMap;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    ActorReference, CategoryPolicy, CoverageOutcome, ManagementDisposition,
    ReferencedResourceConfig, ReferencedResourceType, SecurityCategoryV2, SecurityConfig,
    SecurityReviewerConfigV2, SecurityReviewerOptionsConfigV2,
};
use ward::github::Client;
use ward::github::security::SecurityAndAnalysisState;
use ward::reconcile::security_rules::{
    SecurityCollection, collect_security_category, plan_security_category, verify_security_category,
};

fn managed_sensitive_policy() -> CategoryPolicy {
    CategoryPolicy {
        disposition: ManagementDisposition::Managed,
        prune: false,
        sensitive: true,
    }
}

fn base_security_category() -> SecurityCategoryV2 {
    SecurityCategoryV2::observe_sensitive()
}

fn empty_security_collection() -> SecurityCollection {
    SecurityCollection {
        repository_id: 42,
        category: base_security_category(),
        analysis: SecurityAndAnalysisState::default(),
        private_vulnerability_reporting: None,
        codeql_default_setup: None,
        attached_configuration: None,
        available_configurations: Vec::new(),
        team_ids_by_slug: HashMap::from([("platform".to_owned(), 11)]),
        repository_role_ids_by_name: HashMap::from([("admin".to_owned(), 5)]),
        coverage: Vec::new(),
        issues: Vec::new(),
    }
}

#[tokio::test]
async fn security_rules_permission_degradation_collects_warning_instead_of_failing() {
    let server = MockServer::start().await;
    let repo = json!({
        "id": 42,
        "name": "example",
        "full_name": "test-org/example",
        "archived": false,
        "default_branch": "main",
        "visibility": "private",
        "security_and_analysis": {
            "secret_scanning": { "status": "enabled" },
            "secret_scanning_ai_detection": { "status": "enabled" },
            "secret_scanning_push_protection": { "status": "enabled" },
            "advanced_security": { "status": "enabled" }
        }
    });

    Mock::given(method("GET"))
        .and(path("/repos/test-org/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/automated-security-fixes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "enabled": true })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/example/private-vulnerability-reporting",
        ))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "message": "Forbidden" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-scanning/default-setup"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-security-configuration"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "message": "Forbidden" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/code-security/configurations"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "message": "Forbidden" })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_security_category(&client, "example", None)
        .await
        .unwrap();

    assert_eq!(collected.repository_id, 42);
    assert!(
        collected
            .category
            .settings
            .as_ref()
            .unwrap()
            .secret_scanning
    );
    assert_eq!(collected.category.private_vulnerability_reporting, None);
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "GET /repos/{owner}/{repo}/code-security-configuration"
            && entry.outcome == CoverageOutcome::PermissionDenied
    }));
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "GET /repos/{owner}/{repo}/private-vulnerability-reporting"
            && entry.outcome == CoverageOutcome::PermissionDenied
    }));
}

#[tokio::test]
async fn security_rules_attached_configuration_precedence_ignores_direct_settings() {
    let server = MockServer::start().await;
    let repo = json!({
        "id": 42,
        "name": "example",
        "full_name": "test-org/example",
        "archived": false,
        "default_branch": "main",
        "visibility": "private",
        "security_and_analysis": {
            "secret_scanning": { "status": "enabled" },
            "secret_scanning_ai_detection": { "status": "enabled" },
            "secret_scanning_push_protection": { "status": "enabled" }
        }
    });

    Mock::given(method("GET"))
        .and(path("/repos/test-org/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/automated-security-fixes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "enabled": true })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/example/private-vulnerability-reporting",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-scanning/default-setup"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-security-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "attached",
            "configuration": { "id": 7, "name": "baseline", "target_type": "organization" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/code-security/configurations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 7, "name": "baseline", "target_type": "organization" },
            { "id": 8, "name": "strict", "target_type": "organization" }
        ])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_security_category(&client, "example", None)
        .await
        .unwrap();
    let desired = SecurityCategoryV2 {
        policy: managed_sensitive_policy(),
        settings: Some(SecurityConfig {
            secret_scanning: false,
            secret_scanning_ai_detection: false,
            push_protection: false,
            dependabot_alerts: false,
            dependabot_security_updates: false,
            codeql_advanced_setup: false,
        }),
        advanced_security: None,
        code_security: None,
        dependabot_alerts: None,
        dependabot_security_updates: None,
        secret_scanning: None,
        secret_scanning_push_protection: None,
        secret_scanning_validity_checks: None,
        secret_scanning_non_provider_patterns: None,
        secret_scanning_ai_detection: None,
        secret_scanning_delegated_alert_dismissal: None,
        secret_scanning_delegated_bypass: None,
        secret_scanning_delegated_alert_dismissal_options: None,
        secret_scanning_delegated_bypass_options: None,
        private_vulnerability_reporting: Some(false),
        codeql_default_setup: None,
        configuration_reference: Some(ReferencedResourceConfig {
            resource_type: ReferencedResourceType::CodeSecurityConfiguration,
            name: "strict".to_owned(),
        }),
        delegated_alert_dismissal_reviewers: Vec::new(),
        delegated_bypass_reviewers: Vec::new(),
        references: Vec::new(),
    };

    let plan = plan_security_category(&desired, &collected).unwrap();

    assert_eq!(plan.attach_configuration_id, Some(8));
    assert!(plan.patch_security_and_analysis.is_none());
    assert!(plan.dependabot_alerts.is_none());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.code == "security-attached-configuration-precedence")
    );
}

#[tokio::test]
async fn matching_attached_configuration_verifies_without_sensitive_policy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 42,
            "name": "example",
            "full_name": "test-org/example",
            "archived": false,
            "default_branch": "main",
            "visibility": "private",
            "security_and_analysis": {}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/automated-security-fixes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "enabled": true })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/example/private-vulnerability-reporting",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-scanning/default-setup"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-security-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "attached",
            "configuration": {
                "id": 7,
                "name": "baseline",
                "target_type": "organization"
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/code-security/configurations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 7,
            "name": "baseline",
            "target_type": "organization"
        }])))
        .mount(&server)
        .await;

    let desired = SecurityCategoryV2 {
        policy: CategoryPolicy::managed(),
        configuration_reference: Some(ReferencedResourceConfig {
            resource_type: ReferencedResourceType::CodeSecurityConfiguration,
            name: "baseline".to_owned(),
        }),
        ..base_security_category()
    };
    let client = Client::new_for_test("test-org", &server.uri());

    let verification = verify_security_category(&client, "example", &desired)
        .await
        .unwrap();

    assert!(verification.matches);
    assert!(!verification.plan.has_changes());
    assert!(
        verification
            .plan
            .issues
            .iter()
            .all(|issue| issue.code != "security-sensitive-gate")
    );
}

#[tokio::test]
async fn security_rules_denied_dependabot_alerts_stay_unknown() {
    let server = MockServer::start().await;
    let repo = json!({
        "id": 42,
        "name": "example",
        "full_name": "test-org/example",
        "archived": false,
        "default_branch": "main",
        "visibility": "private",
        "security_and_analysis": {
            "advanced_security": { "status": "enabled" }
        }
    });

    Mock::given(method("GET"))
        .and(path("/repos/test-org/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "message": "Forbidden" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/automated-security-fixes"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/example/private-vulnerability-reporting",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-scanning/default-setup"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-security-configuration"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_security_category(&client, "example", None)
        .await
        .unwrap();

    assert_eq!(collected.category.dependabot_alerts, None);
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "GET /repos/{owner}/{repo}/vulnerability-alerts"
            && entry.outcome == CoverageOutcome::PermissionDenied
    }));
}

#[tokio::test]
async fn security_rules_collects_delegated_bypass_reviewers_with_stable_identity_and_mode() {
    let server = MockServer::start().await;
    let repo = json!({
        "id": 42,
        "name": "example",
        "full_name": "test-org/example",
        "archived": false,
        "default_branch": "main",
        "visibility": "private",
        "security_and_analysis": {
            "advanced_security": { "status": "enabled" },
            "code_security": { "status": "enabled" },
            "secret_scanning": { "status": "enabled" },
            "secret_scanning_delegated_bypass": { "status": "enabled" },
            "secret_scanning_delegated_bypass_options": {
                "reviewers": [
                    { "reviewer_id": 11, "reviewer_type": "TEAM", "mode": "ALWAYS" },
                    { "reviewer_id": 5, "reviewer_type": "ROLE", "mode": "EXEMPT" }
                ]
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/repos/test-org/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/automated-security-fixes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "enabled": true })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/example/private-vulnerability-reporting",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-scanning/default-setup"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/code-security-configuration"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 11, "slug": "platform", "name": "Platform" }
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

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_security_category(&client, "example", None)
        .await
        .unwrap();

    let reviewers = &collected
        .category
        .secret_scanning_delegated_bypass_options
        .as_ref()
        .unwrap()
        .reviewers;
    assert!(reviewers.iter().any(|reviewer| reviewer.actor
        == ActorReference::Team {
            slug: "platform".to_owned()
        }
        && reviewer.mode.as_deref() == Some("ALWAYS")));
    assert!(reviewers.iter().any(|reviewer| reviewer.actor
        == ActorReference::Role {
            name: "admin".to_owned()
        }
        && reviewer.mode.as_deref() == Some("EXEMPT")));
}

#[test]
fn security_rules_plan_never_patches_unsupported_repo_security_fields() {
    let mut actual = empty_security_collection();
    actual.category.code_security = Some(false);
    actual.category.secret_scanning_validity_checks = Some(false);
    actual
        .category
        .secret_scanning_delegated_alert_dismissal_options =
        Some(SecurityReviewerOptionsConfigV2 {
            reviewers: vec![SecurityReviewerConfigV2 {
                actor: ActorReference::Team {
                    slug: "platform".to_owned(),
                },
                mode: Some("ALWAYS".to_owned()),
            }],
        });

    let desired = SecurityCategoryV2 {
        policy: managed_sensitive_policy(),
        code_security: Some(true),
        secret_scanning_validity_checks: Some(true),
        secret_scanning_delegated_alert_dismissal_options: Some(SecurityReviewerOptionsConfigV2 {
            reviewers: vec![SecurityReviewerConfigV2 {
                actor: ActorReference::Role {
                    name: "admin".to_owned(),
                },
                mode: Some("EXEMPT".to_owned()),
            }],
        }),
        ..base_security_category()
    };

    let plan = plan_security_category(&desired, &actual).unwrap();
    let patch = plan.patch_security_and_analysis.unwrap();

    assert_eq!(patch["code_security"], json!({ "status": "enabled" }));
    assert!(patch.get("secret_scanning_validity_checks").is_none());
    assert!(
        patch
            .get("secret_scanning_delegated_alert_dismissal_options")
            .is_none()
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.code == "security-unsupported-validity-checks")
    );
    assert!(
        plan.issues.iter().any(|issue| {
            issue.code == "security-unsupported-delegated-alert-dismissal-options"
        })
    );
}

#[test]
fn security_rules_plan_uses_current_documented_bypass_patch_schema() {
    let actual = empty_security_collection();
    let desired = SecurityCategoryV2 {
        policy: managed_sensitive_policy(),
        secret_scanning_delegated_bypass_options: Some(SecurityReviewerOptionsConfigV2 {
            reviewers: vec![
                SecurityReviewerConfigV2 {
                    actor: ActorReference::Team {
                        slug: "platform".to_owned(),
                    },
                    mode: Some("always".to_owned()),
                },
                SecurityReviewerConfigV2 {
                    actor: ActorReference::Role {
                        name: "admin".to_owned(),
                    },
                    mode: Some("EXEMPT".to_owned()),
                },
            ],
        }),
        ..base_security_category()
    };

    let plan = plan_security_category(&desired, &actual).unwrap();
    let patch = plan.patch_security_and_analysis.unwrap();

    assert_eq!(
        patch["secret_scanning_delegated_bypass"],
        json!({ "status": "enabled" })
    );
    assert_eq!(
        patch["secret_scanning_delegated_bypass_options"]["reviewers"],
        json!([
            {
                "reviewer_id": 11,
                "reviewer_type": "TEAM",
                "mode": "ALWAYS"
            },
            {
                "reviewer_id": 5,
                "reviewer_type": "ROLE",
                "mode": "EXEMPT"
            }
        ])
    );
}
