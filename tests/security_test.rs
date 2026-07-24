mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;
use ward::github::dependency_graph::DependencyGraphStatus;

#[tokio::test]
async fn test_get_security_state_all_enabled() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/automated-security-fixes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "enabled": true, "paused": false })),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "my-repo",
            "security_and_analysis": {
                "secret_scanning": { "status": "enabled" },
                "secret_scanning_ai_detection": { "status": "enabled" },
                "secret_scanning_push_protection": { "status": "enabled" }
            }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let state = client.get_security_state("my-repo").await.unwrap();

    assert!(state.dependabot_alerts);
    assert!(state.dependabot_security_updates);
    assert!(state.secret_scanning);
    assert!(state.secret_scanning_ai_detection);
    assert!(state.push_protection);
}

#[tokio::test]
async fn test_get_security_state_all_disabled() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/automated-security-fixes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "enabled": false, "paused": false })),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "my-repo",
            "security_and_analysis": {
                "secret_scanning": { "status": "disabled" },
                "secret_scanning_ai_detection": { "status": "disabled" },
                "secret_scanning_push_protection": { "status": "disabled" }
            }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let state = client.get_security_state("my-repo").await.unwrap();

    assert!(!state.dependabot_alerts);
    assert!(!state.dependabot_security_updates);
    assert!(!state.secret_scanning);
    assert!(!state.secret_scanning_ai_detection);
    assert!(!state.push_protection);
}

#[tokio::test]
async fn test_enable_dependabot_alerts() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/vulnerability-alerts"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client.enable_dependabot_alerts("my-repo").await.unwrap();
}

#[tokio::test]
async fn test_set_security_features() {
    let server = MockServer::start().await;

    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/my-repo"))
        .and(body_partial_json(json!({
            "security_and_analysis": {
                "secret_scanning": { "status": "enabled" },
                "secret_scanning_ai_detection": { "status": "enabled" },
                "secret_scanning_push_protection": { "status": "enabled" }
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "my-repo"})))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .set_security_features("my-repo", true, true, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_detach_code_security_configurations_sends_json_through_shared_client() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/orgs/test-org/code-security/configurations/detach"))
        .and(body_partial_json(json!({
            "selected_repository_ids": [11, 22]
        })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .detach_code_security_configurations(&[11, 22])
        .await
        .expect("detach request should succeed");
}

#[tokio::test]
async fn test_audit_dependency_graph_available() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/dependency-graph/sbom"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sbom": {
                "creationInfo": { "created": "2026-04-19T10:00:00Z" },
                "packages": [
                    { "SPDXID": "SPDXRef-Repository" },
                    { "SPDXID": "SPDXRef-Package-1" },
                    { "SPDXID": "SPDXRef-Package-2" }
                ]
            }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let audit = client.audit_dependency_graph("my-repo").await;

    assert_eq!(audit.status, DependencyGraphStatus::Available);
    assert_eq!(audit.package_count, Some(3));
    assert_eq!(audit.dependency_count, Some(2));
    assert_eq!(
        audit.sbom_generated_at.as_deref(),
        Some("2026-04-19T10:00:00Z")
    );
}

#[tokio::test]
async fn test_audit_dependency_graph_unavailable() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/dependency-graph/sbom"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let audit = client.audit_dependency_graph("my-repo").await;

    assert_eq!(audit.status, DependencyGraphStatus::Unavailable);
    assert_eq!(
        audit.reason,
        "GitHub could not export an SBOM for this repository"
    );
}
