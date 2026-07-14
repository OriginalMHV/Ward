use serde_json::json;
use ward::config::manifest::{
    ReferencedResourceConfig, ReferencedResourceType, RepositoryAccessCategoryV2,
    RepositoryIntegrationsCategoryV2,
};
use ward::github::Client;
use ward::github::actions::WriteOutcome;
use ward::reconcile::access_integrations::{collect_access, collect_integrations};
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn collect_access_degrades_on_partial_403_and_uses_documented_app_lookup() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/teams"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(vec![json!({
            "id": 1,
            "name": "Platform",
            "slug": "platform",
            "permission": "push",
            "privacy": "closed"
        })]))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/collaborators"))
        .and(query_param("affiliation", "direct"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "forbidden"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/collaborators"))
        .and(query_param("affiliation", "outside"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/invitations"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "installations": [{"id": 77, "app_slug": "deploy-protect"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations/77/repositories"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "repositories": [{"id": 10, "name": "my-repo"}]
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let desired = RepositoryAccessCategoryV2 {
        references: vec![ReferencedResourceConfig {
            resource_type: ReferencedResourceType::App,
            name: "deploy-protect".to_owned(),
        }],
        ..RepositoryAccessCategoryV2::default()
    };

    let collection = collect_access(&client, "my-repo", &desired).await.unwrap();
    assert_eq!(collection.state.teams.len(), 1);
    assert!(!collection.state.collaborators_complete);
    assert!(
        collection
            .coverage
            .iter()
            .any(|entry| entry.endpoint.contains("affiliation=direct"))
    );
    assert_eq!(collection.state.references[0].present, Some(true));
}

#[tokio::test]
async fn collect_access_discovers_source_app_references_when_desired_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/teams"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/collaborators"))
        .and(query_param("affiliation", "direct"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/collaborators"))
        .and(query_param("affiliation", "outside"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/invitations"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "installations": [
                {"id": 11, "app_slug": "deploy-protect"},
                {"id": 12, "app_slug": "audit-bot"}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations/11/repositories"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "repositories": [{"id": 1, "name": "my-repo"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations/12/repositories"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "repositories": [{"id": 2, "name": "other-repo"}]
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collection = collect_access(&client, "my-repo", &RepositoryAccessCategoryV2::default())
        .await
        .unwrap();

    assert_eq!(
        collection.category.references,
        vec![ReferencedResourceConfig {
            resource_type: ReferencedResourceType::App,
            name: "deploy-protect".to_owned(),
        }]
    );
}

#[tokio::test]
async fn access_mutations_encode_login_team_and_secret_names() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/collaborators/user%2Fname"))
        .and(body_partial_json(json!({ "permission": "push" })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/orgs/test-org/teams/team%2Fslug/repos/test-org/my-repo",
        ))
        .and(body_partial_json(json!({ "permission": "push" })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 999})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/orgs/test-org/actions/secrets/NAME%2FWITH%20SPACE/repositories/999",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .add_repo_collaborator("my-repo", "user/name", "push")
        .await
        .unwrap();
    client
        .add_team_to_repo("my-repo", "team/slug", "push")
        .await
        .unwrap();
    let outcome = client
        .associate_org_secret_with_repo("NAME/WITH SPACE", "my-repo")
        .await
        .unwrap();
    assert_eq!(outcome, WriteOutcome::Applied(()));
}

#[tokio::test]
async fn collect_integrations_imports_credentialed_webhook_placeholder_and_autolink_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/hooks"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(vec![json!({
            "id": 4,
            "active": true,
            "events": ["push"],
            "config": {
                "url": "https://user:pass@hooks.example.test/events",
                "content_type": "json",
                "insecure_ssl": "0"
            }
        })]))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/keys"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/pages"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "not found"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/autolinks"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(vec![json!({
            "id": 9,
            "key_prefix": "ABC-",
            "url_template": "https://tracker.example/ABC-<num>",
            "is_alphanumeric": false
        })]))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let current = collect_integrations(
        &client,
        "my-repo",
        &RepositoryIntegrationsCategoryV2::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        current.category.webhooks[0].url,
        "https://***@hooks.example.test/events"
    );
    assert!(matches!(
        current.category.webhooks[0].url_from.as_ref(),
        Some(ward::config::manifest::ExternalValueReference::Env { key })
            if key.starts_with("WARD_WEBHOOK_URL_")
    ));
    assert_eq!(current.category.autolinks[0].is_alphanumeric, Some(false));
    assert!(!current.state.pages_complete);
}
