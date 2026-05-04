mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;

#[tokio::test]
async fn test_list_org_teams() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/orgs/test-org/teams"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 1,
                "name": "DevOps",
                "slug": "devops",
                "description": "Platform team",
                "permission": "push",
                "privacy": "closed"
            },
            {
                "id": 2,
                "name": "Backend",
                "slug": "backend",
                "description": "Backend team",
                "permission": "push",
                "privacy": "closed"
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/orgs/test-org/teams"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let teams = client.list_org_teams().await.unwrap();

    assert_eq!(teams.len(), 2);
    assert_eq!(teams[0].slug, "devops");
    assert_eq!(teams[1].slug, "backend");
}

#[tokio::test]
async fn test_list_repo_teams() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 1,
                "name": "DevOps",
                "slug": "devops",
                "description": "Platform team",
                "permission": "admin",
                "privacy": "closed"
            }
        ])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let teams = client.list_repo_teams("my-repo").await.unwrap();

    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].slug, "devops");
    assert_eq!(teams[0].permission, "admin");
}

#[tokio::test]
async fn test_add_team_to_repo() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/orgs/test-org/teams/devops/repos/test-org/my-repo"))
        .and(body_partial_json(json!({ "permission": "push" })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .add_team_to_repo("my-repo", "devops", "push")
        .await
        .unwrap();
}
