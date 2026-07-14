mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;
use ward::github::settings::{ClassifiedApiResponse, GraphqlRepositoryPatch};

#[tokio::test]
async fn reads_repository_settings_and_topics() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "has_issues": true,
            "has_projects": false,
            "has_wiki": false,
            "has_discussions": true,
            "allow_squash_merge": true,
            "allow_merge_commit": false,
            "allow_rebase_merge": true,
            "allow_auto_merge": true,
            "delete_branch_on_merge": true,
            "allow_update_branch": true,
            "squash_merge_commit_title": "PR_TITLE",
            "squash_merge_commit_message": "PR_BODY",
            "merge_commit_title": "PR_TITLE",
            "merge_commit_message": "PR_BODY",
            "web_commit_signoff_required": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/topics"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "names": ["managed"] })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let settings = client.get_settings("my-repo").await.unwrap();
    let topics = client.get_topics("my-repo").await.unwrap();

    assert!(settings.allow_auto_merge);
    assert!(settings.web_commit_signoff_required);
    assert_eq!(topics, vec!["managed"]);
}

#[tokio::test]
async fn updates_repository_settings_and_topics() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/my-repo"))
        .and(body_partial_json(json!({
            "allow_auto_merge": true,
            "delete_branch_on_merge": true
        })))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/topics"))
        .and(body_partial_json(json!({ "names": ["managed"] })))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .update_settings(
            "my-repo",
            &json!({
                "allow_auto_merge": true,
                "delete_branch_on_merge": true
            }),
        )
        .await
        .unwrap();
    client
        .replace_topics("my-repo", &["managed".to_owned()])
        .await
        .unwrap();
}

#[tokio::test]
async fn reads_general_repository_and_graphql_settings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "node_id": "R_kgDOTest",
            "description": "Managed repo",
            "homepage": "https://example.test",
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
            "data": {
                "repository": {
                    "hasDiscussionsEnabled": true,
                    "hasSponsorshipsEnabled": false,
                    "issueCreationPolicy": "ALL"
                }
            }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let rest = client
        .get_repository_general_settings("my-repo")
        .await
        .unwrap();
    let graphql = client
        .get_repository_graphql_settings("my-repo")
        .await
        .unwrap();

    assert_eq!(rest.node_id, "R_kgDOTest");
    assert_eq!(rest.pull_request_creation_policy.as_deref(), Some("all"));
    assert!(graphql.has_discussions_enabled);
    assert!(!graphql.has_sponsorships_enabled);
    assert_eq!(graphql.issue_creation_policy.as_deref(), Some("ALL"));
}

#[tokio::test]
async fn updates_graphql_repository_settings() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_partial_json(json!({
            "variables": {
                "input": {
                    "repositoryId": "R_kgDOTest",
                    "hasDiscussionsEnabled": true,
                    "hasSponsorshipsEnabled": true,
                    "issueCreationPolicy": "COLLABORATORS_ONLY"
                }
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "updateRepository": {
                    "repository": {
                        "hasDiscussionsEnabled": true,
                        "hasSponsorshipsEnabled": true,
                        "issueCreationPolicy": "COLLABORATORS_ONLY"
                    }
                }
            }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let updated = client
        .update_repository_graphql_settings(
            "R_kgDOTest",
            &GraphqlRepositoryPatch {
                has_discussions_enabled: Some(true),
                has_sponsorships_enabled: Some(true),
                issue_creation_policy: Some("collaborators_only".to_owned()),
            },
        )
        .await
        .unwrap();

    assert!(updated.has_discussions_enabled);
    assert!(updated.has_sponsorships_enabled);
    assert_eq!(
        updated.issue_creation_policy.as_deref(),
        Some("COLLABORATORS_ONLY")
    );
}

#[tokio::test]
async fn classifies_custom_property_permission_missing_and_validation_responses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/forbidden/properties/values"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Resource not accessible by integration"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/missing/properties/values"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/invalid/properties/values"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Validation Failed"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());

    let forbidden = client
        .get_custom_property_values("forbidden")
        .await
        .unwrap();
    let missing = client.get_custom_property_values("missing").await.unwrap();
    let invalid = client
        .update_custom_property_values("invalid", &[])
        .await
        .unwrap();

    assert!(matches!(forbidden, ClassifiedApiResponse::Forbidden(_)));
    assert!(matches!(missing, ClassifiedApiResponse::NotFound(_)));
    assert!(matches!(invalid, ClassifiedApiResponse::Unprocessable(_)));
}

#[tokio::test]
async fn updates_custom_property_array_values() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/my-repo/properties/values"))
        .and(body_partial_json(json!({
            "properties": [
                {
                    "property_name": "systems",
                    "value": ["ward", "party"]
                }
            ]
        })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let result = client
        .update_custom_property_values(
            "my-repo",
            &[ward::github::settings::CustomPropertyValueMutation {
                property_name: "systems".to_owned(),
                value: Some(json!(["ward", "party"])),
            }],
        )
        .await
        .unwrap();

    assert!(matches!(
        result,
        ClassifiedApiResponse::Success(()) | ClassifiedApiResponse::NoContent
    ));
}

#[tokio::test]
async fn encodes_label_and_branch_path_segments() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/my-repo/labels/release%2Fready%20now"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "release/ready now",
            "color": "0052cc",
            "description": "Encoded label",
            "default": false
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/my-repo/labels/release%2Fready%20now"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/branches/release%2F2026.07"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "release/2026.07"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .update_label(
            "my-repo",
            "release/ready now",
            None,
            Some("0052cc"),
            Some("Encoded label"),
        )
        .await
        .unwrap();
    client
        .delete_label("my-repo", "release/ready now")
        .await
        .unwrap();

    assert!(
        client
            .branch_exists("my-repo", "release/2026.07")
            .await
            .unwrap()
    );
}
