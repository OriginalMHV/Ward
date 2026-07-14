mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::CoverageOutcome;
use ward::github::Client;
use ward::reconcile::general::collect;

#[tokio::test]
async fn collects_general_repository_state_with_partial_optional_failures() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "node_id": "R_kgDOTest",
            "description": null,
            "homepage": null,
            "default_branch": "main",
            "visibility": "private",
            "archived": false,
            "is_template": false,
            "allow_forking": true,
            "has_issues": true,
            "has_projects": false,
            "has_wiki": true,
            "has_discussions": true,
            "has_pull_requests": true,
            "pull_request_creation_policy": "all",
            "allow_squash_merge": true,
            "allow_merge_commit": false,
            "allow_rebase_merge": true,
            "allow_auto_merge": true,
            "delete_branch_on_merge": true,
            "allow_update_branch": true,
            "use_squash_pr_title_as_default": false,
            "squash_merge_commit_title": "PR_TITLE",
            "squash_merge_commit_message": "PR_BODY",
            "merge_commit_title": "PR_TITLE",
            "merge_commit_message": "PR_BODY",
            "web_commit_signoff_required": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_partial_json(json!({
            "variables": {
                "owner": "test-org",
                "name": "my-repo"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "errors": [
                { "message": "GraphQL access denied" }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/topics"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Forbidden"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/properties/values"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "property_name": "systems",
                "value": ["ward", "party"]
            },
            {
                "property_name": "team",
                "value": "platform"
            }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/immutable-releases"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Admin permission required"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/labels"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect(&client, "my-repo").await.unwrap();

    let metadata = collected.repository.metadata.as_ref().unwrap();
    let settings = collected.repository.settings.as_ref().unwrap();

    assert_eq!(metadata.description.as_deref(), Some(""));
    assert_eq!(metadata.homepage.as_deref(), Some(""));
    assert!(settings.topics.is_none());
    assert_eq!(collected.custom_properties.len(), 2);
    assert_eq!(collected.custom_properties[0].property_name, "systems");
    assert_eq!(
        collected.custom_properties[0].value,
        json!(["ward", "party"])
    );
    assert!(collected.extensions.use_squash_pr_title_as_default == Some(false));
    assert!(!collected.extensions.graphql_settings_collected);
    assert!(!collected.extensions.labels_collected);
    assert!(!collected.extensions.immutable_releases_collected);
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "POST /graphql repository settings"
            && entry.outcome == CoverageOutcome::Unavailable
    }));
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "GET /repos/{owner}/{repo}/topics"
            && entry.outcome == CoverageOutcome::PermissionDenied
    }));
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "GET /repos/{owner}/{repo}/labels"
            && entry.outcome == CoverageOutcome::NotApplicable
    }));
}
