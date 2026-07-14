mod common;

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;

use common::make_repo_json;

#[tokio::test]
async fn test_list_repos_single_page() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/orgs/test-org/repos"))
        .and(query_param("per_page", "100"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_repo_json("alpha", false),
            make_repo_json("beta", false),
            make_repo_json("gamma", false),
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/orgs/test-org/repos"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = client.list_repos().await.unwrap();

    assert_eq!(repos.len(), 3);
    assert_eq!(repos[0].name, "alpha");
    assert_eq!(repos[1].name, "beta");
    assert_eq!(repos[2].name, "gamma");
}

#[tokio::test]
async fn test_list_repos_pagination() {
    let server = MockServer::start().await;

    // Page 1: 100 repos
    let page1: Vec<serde_json::Value> = (0..100)
        .map(|i| make_repo_json(&format!("repo-{i:03}"), false))
        .collect();
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/repos"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(page1)))
        .mount(&server)
        .await;

    // Page 2: 50 repos
    let page2: Vec<serde_json::Value> = (100..150)
        .map(|i| make_repo_json(&format!("repo-{i:03}"), false))
        .collect();
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/repos"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(page2)))
        .mount(&server)
        .await;

    // Page 3: empty
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/repos"))
        .and(query_param("page", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = client.list_repos().await.unwrap();

    assert_eq!(repos.len(), 150);
    assert_eq!(repos[0].name, "repo-000");
    assert_eq!(repos[149].name, "repo-149");
}

#[tokio::test]
async fn test_list_repos_includes_archived() {
    let server = MockServer::start().await;

    // list_repos itself does NOT filter archived — that happens in list_repos_for_system
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/repos"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_repo_json("active-one", false),
            make_repo_json("archived-one", true),
            make_repo_json("active-two", false),
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/orgs/test-org/repos"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = client.list_repos().await.unwrap();

    // list_repos returns ALL repos including archived
    assert_eq!(repos.len(), 3);
    assert!(repos[1].archived);
}

#[tokio::test]
async fn test_list_repos_for_system_with_prefix() {
    let server = MockServer::start().await;

    // list_repos_for_system now uses the search API instead of listing all repos
    Mock::given(method("GET"))
        .and(path("/search/repositories"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 4,
            "items": [
                make_repo_json("frontend-foo", false),
                make_repo_json("frontend-bar", false),
                make_repo_json("webapp-baz", false),
                make_repo_json("frontend-archived", true),
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/search/repositories"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 4,
            "items": []
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = client
        .list_repos_for_system("frontend", true, &[], &[])
        .await
        .unwrap();

    assert_eq!(repos.len(), 2);
    assert!(repos.iter().all(|r| r.name.starts_with("frontend")));
    assert!(repos.iter().all(|r| !r.archived));
}

#[tokio::test]
async fn test_list_repos_for_system_rejects_partial_prefix() {
    let server = MockServer::start().await;

    // Search returns repos that GitHub matches broadly — our code filters further
    Mock::given(method("GET"))
        .and(path("/search/repositories"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 3,
            "items": [
                make_repo_json("be-api", false),
                make_repo_json("be-frontend", false),
                make_repo_json("backend-service", false),
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/search/repositories"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 3,
            "items": []
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = client
        .list_repos_for_system("be", true, &[], &[])
        .await
        .unwrap();

    // "backend-service" should NOT match system "be" — only "be-api" and "be-frontend"
    assert_eq!(repos.len(), 2);
    let names: Vec<&str> = repos.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"be-api"));
    assert!(names.contains(&"be-frontend"));
    assert!(!names.contains(&"backend-service"));
}

#[tokio::test]
async fn explicit_only_system_skips_repository_search() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/reference"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_repo_json("reference", false)))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = client
        .list_repos_for_system("reference", false, &[], &["reference".to_owned()])
        .await
        .unwrap();

    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "reference");
}
