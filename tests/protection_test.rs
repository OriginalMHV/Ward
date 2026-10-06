mod common;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;

#[tokio::test]
async fn test_get_branch_protection_exists() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "required_pull_request_reviews": {
                "required_approving_review_count": 2,
                "dismiss_stale_reviews": true,
                "require_code_owner_reviews": true
            },
            "required_status_checks": {
                "strict": true,
                "contexts": []
            },
            "enforce_admins": { "enabled": true },
            "required_linear_history": { "enabled": false },
            "allow_force_pushes": { "enabled": false },
            "allow_deletions": { "enabled": false }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let protection = client
        .get_branch_protection("my-repo", "main")
        .await
        .unwrap();

    let state = protection.expect("should return Some");
    assert!(state.required_pull_request_reviews);
    assert_eq!(state.required_approving_review_count, 2);
    assert!(state.dismiss_stale_reviews);
    assert!(state.require_code_owner_reviews);
    assert!(state.required_status_checks);
    assert!(state.strict_status_checks);
    assert!(state.enforce_admins);
    assert!(!state.required_linear_history);
    assert!(!state.allow_force_pushes);
    assert!(!state.allow_deletions);
}

#[tokio::test]
async fn test_get_branch_protection_none() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Branch not protected"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let protection = client
        .get_branch_protection("my-repo", "main")
        .await
        .unwrap();

    assert!(protection.is_none());
}

#[tokio::test]
async fn test_update_branch_protection_omits_absent_check_app_id() {
    use ward::github::branch_protection::{DesiredBranchProtection, StatusCheckRequirement};

    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let desired = DesiredBranchProtection {
        required_status_checks: true,
        status_check_contexts: vec!["build".to_owned(), "lint".to_owned()],
        status_checks: vec![
            StatusCheckRequirement {
                context: "build".to_owned(),
                app_id: None,
            },
            StatusCheckRequirement {
                context: "lint".to_owned(),
                app_id: Some(15368),
            },
        ],
        ..DesiredBranchProtection::default()
    };
    let client = Client::new_for_test("test-org", &server.uri());
    client
        .update_branch_protection_detailed("my-repo", "main", &desired)
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["required_status_checks"]["checks"],
        json!([{ "context": "build" }, { "context": "lint", "app_id": 15368 }])
    );
}

#[tokio::test]
async fn empty_review_restrictions_are_omitted_so_user_owned_repositories_accept_the_put() {
    use ward::github::branch_protection::DesiredBranchProtection;

    let server = MockServer::start().await;
    mount_repo_owner(&server, "User").await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let desired = DesiredBranchProtection {
        required_pull_request_reviews: true,
        required_approving_review_count: 1,
        ..DesiredBranchProtection::default()
    };
    let client = Client::new_for_test("test-org", &server.uri());
    client
        .update_branch_protection_detailed("my-repo", "main", &desired)
        .await
        .unwrap();

    let body = put_body(&server).await;
    let reviews = &body["required_pull_request_reviews"];
    assert_eq!(reviews["required_approving_review_count"], 1);
    assert!(reviews.get("dismissal_restrictions").is_none(), "{reviews}");
    assert!(
        reviews.get("bypass_pull_request_allowances").is_none(),
        "{reviews}"
    );
}

#[tokio::test]
async fn organization_only_restriction_errors_name_the_cause() {
    use ward::github::branch_protection::{ActorSet, DesiredBranchProtection, TeamActor};

    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Validation Failed",
            "errors": ["Only organization repositories can have users and team restrictions"]
        })))
        .mount(&server)
        .await;

    let desired = DesiredBranchProtection {
        required_pull_request_reviews: true,
        dismissal_restrictions: ActorSet {
            teams: vec![TeamActor {
                slug: "core".to_owned(),
            }],
            ..ActorSet::default()
        },
        ..DesiredBranchProtection::default()
    };
    let client = Client::new_for_test("test-org", &server.uri());
    let error = client
        .update_branch_protection_detailed("my-repo", "main", &desired)
        .await
        .unwrap_err();
    let message = format!("{error:#}");

    assert!(
        message.contains("require an organization-owned repository"),
        "{message}"
    );
    assert!(
        message.contains("Only organization repositories"),
        "{message}"
    );
}

#[tokio::test]
async fn empty_review_restrictions_are_sent_on_organization_repositories_to_clear_them() {
    use ward::github::branch_protection::DesiredBranchProtection;

    let server = MockServer::start().await;
    mount_repo_owner(&server, "Organization").await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let desired = DesiredBranchProtection {
        required_pull_request_reviews: true,
        required_approving_review_count: 1,
        ..DesiredBranchProtection::default()
    };
    let client = Client::new_for_test("test-org", &server.uri());
    client
        .update_branch_protection_detailed("my-repo", "main", &desired)
        .await
        .unwrap();

    let body = put_body(&server).await;
    let reviews = &body["required_pull_request_reviews"];
    let empty = json!({ "users": [], "teams": [], "apps": [] });
    assert_eq!(reviews["dismissal_restrictions"], empty, "{reviews}");
    assert_eq!(
        reviews["bypass_pull_request_allowances"], empty,
        "{reviews}"
    );
}

async fn mount_repo_owner(server: &MockServer, kind: &str) {
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "my-repo",
            "full_name": "test-org/my-repo",
            "archived": false,
            "default_branch": "main",
            "visibility": "public",
            "owner": { "login": "test-org", "type": kind }
        })))
        .mount(server)
        .await;
}

#[allow(
    clippy::unwrap_used,
    reason = "test helper; a missing PUT should fail the test"
)]
async fn put_body(server: &MockServer) -> serde_json::Value {
    let requests = server.received_requests().await.unwrap();
    let put = requests
        .iter()
        .find(|request| request.method.as_str() == "PUT")
        .unwrap();
    serde_json::from_slice(&put.body).unwrap()
}
