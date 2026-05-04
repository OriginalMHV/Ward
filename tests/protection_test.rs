mod common;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::BranchProtectionConfig;
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
async fn test_update_branch_protection() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": "https://api.github.com/repos/test-org/my-repo/branches/main/protection"
        })))
        .mount(&server)
        .await;

    let config = BranchProtectionConfig {
        enabled: true,
        required_approvals: 1,
        dismiss_stale_reviews: true,
        require_code_owner_reviews: false,
        require_status_checks: true,
        strict_status_checks: true,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .update_branch_protection("my-repo", "main", &config)
        .await
        .unwrap();
}
