use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;

#[tokio::test]
async fn server_error_ignores_rate_limit_reset_header() {
    let server = MockServer::start().await;
    let far_future = (chrono::Utc::now().timestamp() + 3600).to_string();
    Mock::given(method("GET"))
        .and(path("/repos/test-org/a"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", far_future.as_str()),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.get("/repos/test-org/a"),
    )
    .await
    .expect("503 must not wait for the rate limit reset")
    .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn forbidden_with_secondary_rate_limit_body_is_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/b"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "You have exceeded a secondary rate limit. Please wait a few minutes."
        })))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = client.get("/repos/test-org/b").await.unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn ordinary_forbidden_is_not_retried_and_keeps_its_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/c"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Resource not accessible by integration"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = client.get("/repos/test-org/c").await.unwrap();

    assert_eq!(response.status(), 403);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["message"], "Resource not accessible by integration");
}

#[tokio::test]
async fn primary_rate_limit_beyond_cap_is_returned_without_retry() {
    let server = MockServer::start().await;
    let far_future = (chrono::Utc::now().timestamp() + 3600).to_string();
    Mock::given(method("GET"))
        .and(path("/repos/test-org/d"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", far_future.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.get("/repos/test-org/d"),
    )
    .await
    .expect("a reset beyond the cap must not be waited for")
    .unwrap();

    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn post_is_not_retried_on_server_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/test-org/e/issues"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = client
        .post_json("/repos/test-org/e/issues", &json!({ "title": "x" }))
        .await
        .unwrap();

    assert_eq!(response.status(), 503);
}

#[tokio::test]
async fn graphql_mutation_is_not_retried_on_server_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(502))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let result = client
        .graphql::<serde_json::Value, _>("mutation M { x }", &json!({}))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn put_patch_and_delete_are_retried_on_server_errors() {
    let server = MockServer::start().await;
    for verb in ["PUT", "PATCH", "DELETE"] {
        Mock::given(method(verb))
            .and(path("/repos/test-org/f"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method(verb))
            .and(path("/repos/test-org/f"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = Client::new_for_test("test-org", &server.uri());
    let body = json!({});
    assert_eq!(client.put("/repos/test-org/f").await.unwrap().status(), 200);
    assert_eq!(
        client
            .patch_json("/repos/test-org/f", &body)
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client.delete("/repos/test-org/f").await.unwrap().status(),
        200
    );
}

#[tokio::test]
async fn gzip_encoded_responses_are_decompressed() {
    const GZIPPED_OK: [u8; 31] = [
        31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 171, 86, 202, 207, 86, 178, 42, 41, 42, 77, 173, 5, 0,
        144, 95, 212, 167, 11, 0, 0, 0,
    ];
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/g"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "gzip")
                .set_body_raw(GZIPPED_OK.to_vec(), "application/json"),
        )
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let body: serde_json::Value = client
        .get("/repos/test-org/g")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn delete_that_is_404_after_a_server_error_retry_counts_as_deleted() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/d/keys/1"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/d/keys/1"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = client.delete("/repos/test-org/d/keys/1").await.unwrap();

    assert_eq!(response.status(), 204);
}

#[tokio::test]
async fn delete_that_is_404_on_the_first_attempt_stays_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/d/keys/2"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = client.delete("/repos/test-org/d/keys/2").await.unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn forbidden_with_an_exhausted_header_but_a_permission_body_is_not_retried() {
    let server = MockServer::start().await;
    let soon = (chrono::Utc::now().timestamp() + 2).to_string();
    Mock::given(method("GET"))
        .and(path("/repos/test-org/e"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", soon.as_str())
                .set_body_json(json!({"message": "Resource not accessible by integration"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let response = client.get("/repos/test-org/e").await.unwrap();

    assert_eq!(response.status(), 403);
}
