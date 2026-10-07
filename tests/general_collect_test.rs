#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    CategoryPolicy, CoverageOutcome, RepositoryCategory, RepositorySettingsConfig,
};
use ward::github::Client;
use ward::reconcile::general::{
    GeneralChangeKind, GeneralDesiredExtensions, GeneralDesiredState, collect, plan,
};

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
    // A successful read records a Collected entry, and that must still count as collected.
    assert!(collected.extensions.custom_properties_collected);
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

fn read_only_repository_response() -> serde_json::Value {
    // Shape of `GET /repos/{repo}` for a caller without push access: the merge settings are omitted.
    json!({
        "node_id": "R_kgDOTest",
        "description": "desc",
        "homepage": null,
        "default_branch": "main",
        "visibility": "public",
        "archived": false,
        "is_template": false,
        "allow_forking": true,
        "has_issues": true,
        "has_projects": true,
        "has_wiki": true,
        "has_discussions": false,
        "web_commit_signoff_required": false,
        "permissions": { "admin": false, "maintain": false, "push": false, "triage": false, "pull": true }
    })
}

fn full_repository_response() -> serde_json::Value {
    let mut body = read_only_repository_response();
    let object = body.as_object_mut().unwrap();
    for (key, value) in [
        ("allow_squash_merge", json!(false)),
        ("allow_merge_commit", json!(false)),
        ("allow_rebase_merge", json!(false)),
        ("allow_auto_merge", json!(false)),
        ("delete_branch_on_merge", json!(true)),
        ("allow_update_branch", json!(false)),
        ("use_squash_pr_title_as_default", json!(false)),
        ("squash_merge_commit_title", json!("COMMIT_OR_PR_TITLE")),
    ] {
        object.insert(key.to_owned(), value);
    }
    body
}

async fn collect_from(body: serde_json::Value) -> ward::reconcile::general::CollectedGeneralState {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = Client::new_for_test("test-org", &server.uri());
    collect(&client, "my-repo").await.unwrap()
}

fn merge_settings_manifest() -> GeneralDesiredState {
    GeneralDesiredState {
        repository: RepositoryCategory {
            policy: CategoryPolicy::managed(),
            settings: Some(RepositorySettingsConfig {
                allow_squash_merge: Some(true),
                allow_merge_commit: Some(false),
                allow_rebase_merge: Some(true),
                allow_auto_merge: Some(true),
                delete_branch_on_merge: Some(true),
                allow_update_branch: Some(true),
                squash_merge_commit_title: Some("PR_TITLE".to_owned()),
                ..RepositorySettingsConfig::default()
            }),
            metadata: None,
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        },
        labels: Vec::new(),
        custom_properties: Vec::new(),
        extensions: GeneralDesiredExtensions {
            use_squash_pr_title_as_default: Some(true),
            ..GeneralDesiredExtensions::default()
        },
    }
}

#[tokio::test]
async fn merge_settings_hidden_from_read_only_tokens_are_unreadable_not_false() {
    let collected = collect_from(read_only_repository_response()).await;

    assert!(collected.extensions.merge_settings_unreadable);
    let settings = collected.repository.settings.as_ref().unwrap();
    assert_eq!(settings.allow_squash_merge, None);
    assert_eq!(settings.allow_rebase_merge, None);
    assert_eq!(settings.delete_branch_on_merge, None);
    let entry = collected
        .coverage
        .iter()
        .find(|entry| entry.endpoint == "GET /repos/{owner}/{repo} merge settings")
        .unwrap();
    assert_eq!(entry.outcome, CoverageOutcome::PermissionDenied);
    assert!(entry.reason.as_deref().unwrap().contains("push or admin"));

    let planned = plan("my-repo", &merge_settings_manifest(), &collected);

    assert!(!planned.has_actionable_changes());
    assert!(!planned.has_blocked_changes());
    assert!(planned.changes.is_empty());
    assert!(
        planned
            .coverage
            .iter()
            .any(|entry| entry.endpoint == "GET /repos/{owner}/{repo} merge settings")
    );
}

#[tokio::test]
async fn merge_settings_returned_as_null_are_also_unreadable() {
    let mut body = read_only_repository_response();
    for key in [
        "allow_squash_merge",
        "allow_merge_commit",
        "allow_rebase_merge",
    ] {
        body[key] = serde_json::Value::Null;
    }

    let collected = collect_from(body).await;

    assert!(collected.extensions.merge_settings_unreadable);
}

#[tokio::test]
async fn full_responses_still_plan_real_merge_setting_drift() {
    let collected = collect_from(full_repository_response()).await;

    assert!(!collected.extensions.merge_settings_unreadable);
    assert!(
        collected
            .coverage
            .iter()
            .all(|entry| entry.endpoint != "GET /repos/{owner}/{repo} merge settings")
    );

    let planned = plan("my-repo", &merge_settings_manifest(), &collected);

    let mut fields: Vec<&str> = planned
        .changes
        .iter()
        .filter_map(|change| match &change.kind {
            GeneralChangeKind::RestField { field } => Some(field.as_str()),
            _ => None,
        })
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        [
            "allow_auto_merge",
            "allow_rebase_merge",
            "allow_squash_merge",
            "allow_update_branch",
            "squash_merge_commit_title",
            "use_squash_pr_title_as_default",
        ]
    );
    let patch = planned.rest_patch.as_object().unwrap();
    assert_eq!(patch["allow_squash_merge"], json!(true));
}
