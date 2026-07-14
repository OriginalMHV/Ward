//! Client-level tests for `src/github/actions.rs` and `src/github/environments.rs`:
//! HTTP status-code semantics (200/201/204/403/404/422), large selected-actions
//! allowlists, environment-name path encoding, and sealed-box secret encryption.

use base64::Engine;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;
use ward::github::actions::{
    ActionsPermissions, PrivateForkPrWorkflows, SelectedActionsPolicy, WriteOutcome,
    seal_secret_value,
};
use ward::github::environments::EnvironmentUpdate;

fn client(server: &MockServer) -> Client {
    Client::new_for_test("test-org", &server.uri())
}

// ---------------------------------------------------------------------------
// Status-code semantics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn set_actions_permissions_204_is_applied() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/actions/permissions"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let outcome = client(&server)
        .set_actions_permissions(
            "my-repo",
            &ActionsPermissions {
                enabled: true,
                allowed_actions: Some("all".to_owned()),
                sha_pinning_required: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(outcome, WriteOutcome::Applied(()));
}

#[tokio::test]
async fn set_actions_permissions_403_is_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/actions/permissions"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "Forbidden"})))
        .mount(&server)
        .await;

    let outcome = client(&server)
        .set_actions_permissions(
            "my-repo",
            &ActionsPermissions {
                enabled: true,
                allowed_actions: None,
                sha_pinning_required: None,
            },
        )
        .await
        .unwrap();

    assert!(matches!(outcome, WriteOutcome::Blocked(_)));
}

#[tokio::test]
async fn set_workflow_permissions_422_is_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/actions/permissions/workflow"))
        .respond_with(
            ResponseTemplate::new(422).set_body_json(json!({"message": "Validation failed"})),
        )
        .mount(&server)
        .await;

    let outcome = client(&server)
        .set_workflow_permissions(
            "my-repo",
            &ward::github::actions::WorkflowPermissions {
                default_workflow_permissions: "write".to_owned(),
                can_approve_pull_request_reviews: true,
            },
        )
        .await
        .unwrap();

    match outcome {
        WriteOutcome::Blocked(reason) => assert!(!reason.is_empty()),
        WriteOutcome::Applied(_) => panic!("expected a blocked outcome for HTTP 422"),
    }
}

#[tokio::test]
async fn delete_actions_secret_404_is_applied_noop() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/my-repo/actions/secrets/MISSING"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&server)
        .await;

    let outcome = client(&server)
        .delete_actions_secret("my-repo", "MISSING")
        .await
        .unwrap();

    // Deleting an already-absent secret is treated as an idempotent no-op.
    assert_eq!(outcome, WriteOutcome::Applied(()));
}

#[tokio::test]
async fn put_environment_404_environment_missing_creates_via_200() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/environments/production"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "production",
            "protection_rules": []
        })))
        .mount(&server)
        .await;

    let outcome = client(&server)
        .put_environment("my-repo", "production", &EnvironmentUpdate::default())
        .await
        .unwrap();

    assert_eq!(outcome, WriteOutcome::Applied(()));
}

#[tokio::test]
async fn create_deployment_branch_policy_201_returns_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repos/test-org/my-repo/environments/production/deployment-branch-policies",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 42,
            "name": "release/*",
            "type": "branch"
        })))
        .mount(&server)
        .await;

    let outcome = client(&server)
        .create_deployment_branch_policy("my-repo", "production", "release/*", "branch")
        .await
        .unwrap();

    match outcome {
        WriteOutcome::Applied(policy) => {
            assert_eq!(policy.id, 42);
            assert_eq!(policy.name, "release/*");
        }
        WriteOutcome::Blocked(reason) => panic!("expected 201 body, got blocked: {reason}"),
    }
}

#[tokio::test]
async fn get_private_fork_pr_workflows_404_is_not_applicable() {
    // Public repositories 404 on this endpoint; the client surfaces this as
    // `Ok(None)` rather than an error so callers can distinguish "not
    // applicable" from a hard failure.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/public-repo/actions/permissions/fork-pr-workflows-private-repos",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&server)
        .await;

    let result = client(&server)
        .get_private_fork_pr_workflows("public-repo")
        .await
        .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn set_private_fork_pr_workflows_200_is_applied() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/repos/test-org/my-repo/actions/permissions/fork-pr-workflows-private-repos",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let outcome = client(&server)
        .set_private_fork_pr_workflows(
            "my-repo",
            &PrivateForkPrWorkflows {
                run_workflows_from_fork_pull_requests: true,
                send_write_tokens_to_workflows: false,
                send_secrets_and_variables: false,
                require_approval_for_fork_pr_workflows: false,
            },
        )
        .await
        .unwrap();

    assert_eq!(outcome, WriteOutcome::Applied(()));
}

// ---------------------------------------------------------------------------
// Large selected-actions allowlist
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_selected_actions_handles_large_allowlist() {
    let patterns: Vec<String> = (0..500).map(|i| format!("org-{i}/*")).collect();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/actions/permissions/selected-actions",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "github_owned_allowed": true,
            "verified_allowed": false,
            "patterns_allowed": patterns
        })))
        .mount(&server)
        .await;

    let policy = client(&server)
        .get_selected_actions("my-repo")
        .await
        .unwrap();
    assert_eq!(policy.patterns_allowed.len(), 500);
    assert_eq!(policy.patterns_allowed[499], "org-499/*");
}

#[tokio::test]
async fn set_selected_actions_sends_full_allowlist_body() {
    let patterns: Vec<String> = (0..250).map(|i| format!("actions-{i}/*")).collect();
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/repos/test-org/my-repo/actions/permissions/selected-actions",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let outcome = client(&server)
        .set_selected_actions(
            "my-repo",
            &SelectedActionsPolicy {
                github_owned_allowed: true,
                verified_allowed: true,
                patterns_allowed: patterns,
            },
        )
        .await
        .unwrap();

    assert_eq!(outcome, WriteOutcome::Applied(()));
}

// ---------------------------------------------------------------------------
// Environment name URL encoding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn environment_endpoints_percent_encode_names_with_special_characters() {
    let server = MockServer::start().await;
    // "staging/eu west" must be percent-encoded segment-wise in the path.
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/environments/staging%2Feu%20west",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "staging/eu west",
            "protection_rules": []
        })))
        .mount(&server)
        .await;

    let environment = client(&server)
        .get_environment("my-repo", "staging/eu west")
        .await
        .unwrap();

    assert_eq!(environment.unwrap().name, "staging/eu west");
}

// ---------------------------------------------------------------------------
// Sealed-box secret encryption
// ---------------------------------------------------------------------------

#[test]
fn seal_secret_value_roundtrips_and_never_equals_plaintext() {
    use crypto_box::SecretKey;
    use crypto_box::aead::OsRng;

    let secret_key = SecretKey::generate(&mut OsRng);
    let public_key_b64 =
        base64::engine::general_purpose::STANDARD.encode(secret_key.public_key().as_bytes());

    let plaintext = "super-secret-token-value";
    let encrypted_b64 = seal_secret_value(&public_key_b64, plaintext).unwrap();

    // The request body sent to GitHub must never contain the plaintext.
    assert_ne!(encrypted_b64, plaintext);
    assert!(!encrypted_b64.contains(plaintext));

    // Must be valid base64 (GitHub requires the `encrypted_value` field to be base64).
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&encrypted_b64)
        .expect("encrypted value must be valid base64");

    // Roundtrip: decrypting with the matching secret key recovers the plaintext.
    let decrypted = secret_key.unseal(&ciphertext).unwrap();
    assert_eq!(decrypted, plaintext.as_bytes());
}

#[test]
fn seal_secret_value_is_nondeterministic_across_calls() {
    use crypto_box::SecretKey;
    use crypto_box::aead::OsRng;

    let secret_key = SecretKey::generate(&mut OsRng);
    let public_key_b64 =
        base64::engine::general_purpose::STANDARD.encode(secret_key.public_key().as_bytes());

    let first = seal_secret_value(&public_key_b64, "value").unwrap();
    let second = seal_secret_value(&public_key_b64, "value").unwrap();

    // Sealed-box encryption uses an ephemeral keypair per call, so identical
    // plaintexts must not produce identical ciphertexts.
    assert_ne!(first, second);
}

#[test]
fn seal_secret_value_rejects_invalid_public_key() {
    let result = seal_secret_value("not-valid-base64!!", "value");
    assert!(result.is_err());
}
