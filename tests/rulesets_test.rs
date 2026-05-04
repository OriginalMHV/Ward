mod common;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;

#[tokio::test]
async fn test_list_rulesets() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/rulesets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 1, "name": "Branch protection" },
            { "id": 2, "name": "Copilot Code Review" }
        ])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let rulesets = client.list_rulesets("my-repo").await.unwrap();

    assert_eq!(rulesets.len(), 2);
    assert_eq!(rulesets[0].id, 1);
    assert_eq!(rulesets[0].name, "Branch protection");
    assert_eq!(rulesets[1].id, 2);
    assert_eq!(rulesets[1].name, "Copilot Code Review");
}

#[tokio::test]
async fn test_get_ruleset_detail() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/rulesets/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 123,
            "name": "Branch protection",
            "enforcement": "active",
            "target": "branch",
            "rules": [
                {
                    "type": "pull_request",
                    "parameters": {
                        "required_approving_review_count": 1,
                        "dismiss_stale_reviews_on_push": false
                    }
                }
            ],
            "conditions": {
                "ref_name": {
                    "include": ["~DEFAULT_BRANCH"],
                    "exclude": []
                }
            },
            "bypass_actors": []
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let detail = client.get_ruleset("my-repo", 123).await.unwrap();

    assert_eq!(detail.id, 123);
    assert_eq!(detail.name, "Branch protection");
    assert_eq!(detail.enforcement, "active");
    assert_eq!(detail.target, "branch");
    assert_eq!(detail.rules.len(), 1);
    assert_eq!(detail.rules[0].rule_type, "pull_request");
}

#[tokio::test]
async fn test_create_ruleset() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/rulesets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 456,
            "name": "Copilot Code Review",
            "enforcement": "active",
            "target": "branch",
            "rules": [{
                "type": "copilot_code_review",
                "parameters": { "review_on_push": true }
            }],
            "conditions": null,
            "bypass_actors": []
        })))
        .mount(&server)
        .await;

    let body = json!({
        "name": "Copilot Code Review",
        "target": "branch",
        "enforcement": "active",
        "rules": [{
            "type": "copilot_code_review",
            "parameters": { "review_on_push": true }
        }]
    });

    let client = Client::new_for_test("test-org", &server.uri());
    let created = client.create_ruleset("my-repo", &body).await.unwrap();

    assert_eq!(created.id, 456);
    assert_eq!(created.name, "Copilot Code Review");
}
