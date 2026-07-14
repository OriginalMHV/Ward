mod common;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use ward::github::Client;

#[derive(Clone)]
struct SequenceResponder {
    calls: Arc<AtomicUsize>,
}

impl SequenceResponder {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Respond for SequenceResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => ResponseTemplate::new(503).set_body_json(json!({
                "message": "Service Unavailable"
            })),
            _ => ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "viewer": {
                        "login": "octocat"
                    }
                }
            })),
        }
    }
}

#[derive(Clone)]
struct RateLimit403ThenSuccess {
    calls: Arc<AtomicUsize>,
}

impl RateLimit403ThenSuccess {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Respond for RateLimit403ThenSuccess {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => ResponseTemplate::new(403)
                .append_header("retry-after", "0")
                .set_body_json(json!({
                    "message": "rate limited"
                })),
            _ => ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "viewer": {
                        "login": "octocat"
                    }
                }
            })),
        }
    }
}

#[tokio::test]
async fn rest_requests_include_version_and_package_user_agent_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/versioned"))
        .and(header(
            "user-agent",
            format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        ))
        .and(header("x-github-api-version", "2022-11-28"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "versioned",
            "full_name": "test-org/versioned",
            "archived": false,
            "default_branch": "main",
            "visibility": "private"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let repo = client.get_repo("versioned").await.unwrap();

    assert_eq!(repo.name, "versioned");
}

#[tokio::test]
async fn graphql_requests_share_authenticated_client_defaults() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(header(
            "user-agent",
            format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        ))
        .and(header("x-github-api-version", "2022-11-28"))
        .and(body_partial_json(json!({
            "query": "query ViewerLogin { viewer { login } }",
            "variables": { "owner": "test-org" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "viewer": {
                    "login": "octocat"
                }
            }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let data = client
        .graphql::<serde_json::Value, _>(
            "query ViewerLogin { viewer { login } }",
            &json!({ "owner": "test-org" }),
        )
        .await
        .unwrap();

    assert_eq!(data["viewer"]["login"], "octocat");
}

#[tokio::test]
async fn graphql_retries_transient_failures_and_reuses_cloneable_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(SequenceResponder::new())
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let data = client
        .graphql::<serde_json::Value, _>(
            "query ViewerLogin { viewer { login } }",
            &json!({ "owner": "test-org" }),
        )
        .await
        .unwrap();

    assert_eq!(data["viewer"]["login"], "octocat");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn validation_failures_do_not_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/no-retry"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Validation Failed"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let error = client
        .get_repo("no-retry")
        .await
        .expect_err("422 must not retry");

    assert!(format!("{error:#}").contains("Validation Failed"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn graphql_retries_rate_limited_forbidden_responses() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(RateLimit403ThenSuccess::new())
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let data = client
        .graphql::<serde_json::Value, _>(
            "query ViewerLogin { viewer { login } }",
            &json!({ "owner": "test-org" }),
        )
        .await
        .unwrap();

    assert_eq!(data["viewer"]["login"], "octocat");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn retries_are_bounded_and_return_last_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/exhausted"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "message": "Service Unavailable"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let error = client
        .get_repo("exhausted")
        .await
        .expect_err("transient failures must stop after bounded retries");

    assert!(format!("{error:#}").contains("Service Unavailable"));
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}
