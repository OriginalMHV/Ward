//! Reconcile-layer tests for the Actions category:
//! `collect_actions_category` / `plan_actions_category` / `apply_actions_plan`
//! / `verify_actions_category`.

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    ActionsCategoryV2, ActionsSettingsConfig, CategoryPolicy, CoverageOutcome,
    ExternalValueReference, ManagementDisposition, ManifestCategoryName, NamedValueConfig,
    ReferencedResourceConfig, ReferencedResourceType, SecretPlaceholderConfig, WorkflowStateConfig,
};
use ward::github::Client;
use ward::reconcile::actions_environments::{
    ActionsPlan, ActionsSettingChange, IssueSeverity, OrgReferenceAction, apply_actions_plan,
    collect_actions_category, plan_actions_category, verify_actions_category,
};

fn client(server: &MockServer) -> Client {
    Client::new_for_test("test-org", &server.uri())
}

fn managed_policy(prune: bool) -> CategoryPolicy {
    CategoryPolicy {
        disposition: ManagementDisposition::Managed,
        prune,
        sensitive: false,
    }
}

fn sensitive_managed_policy(prune: bool) -> CategoryPolicy {
    CategoryPolicy {
        disposition: ManagementDisposition::Managed,
        prune,
        sensitive: true,
    }
}

/// Overridable pieces of the full baseline `collect_actions_category` needs.
/// wiremock resolves ties between equal-priority mocks in *registration*
/// order, so per-test variation must be injected here rather than by
/// re-mounting a mock on top of an already-mounted baseline route.
struct BaselineOverrides {
    permissions: Value,
    fork_pr_workflows_status: u16,
    variables: Value,
    secrets: Value,
    workflows: Value,
    organization_variables: Value,
    organization_secrets: Value,
}

impl Default for BaselineOverrides {
    fn default() -> Self {
        Self {
            permissions: json!({"enabled": true, "allowed_actions": "all"}),
            fork_pr_workflows_status: 200,
            variables: json!({"variables": []}),
            secrets: json!({"secrets": []}),
            workflows: json!({"workflows": []}),
            organization_variables: json!({"variables": []}),
            organization_secrets: json!({"secrets": []}),
        }
    }
}

/// Mount a full, self-consistent set of baseline responses for every endpoint
/// `collect_actions_category` unconditionally queries.
async fn mount_actions_baseline(server: &MockServer, repo: &str, overrides: BaselineOverrides) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/actions/permissions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(overrides.permissions))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/permissions/workflow"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "default_workflow_permissions": "read",
            "can_approve_pull_request_reviews": false
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/permissions/artifact-and-log-retention"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"days": 90})))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/cache/retention-limit"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"max_cache_retention_days": 7})),
        )
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/cache/storage-limit"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"max_cache_size_gb": 10})))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/permissions/fork-pr-contributor-approval"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"approval_policy": "first_time_contributors"})),
        )
        .mount(server)
        .await;

    let fork_pr_response = if overrides.fork_pr_workflows_status == 404 {
        ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"}))
    } else {
        ResponseTemplate::new(200).set_body_json(json!({
            "run_workflows_from_fork_pull_requests": true,
            "send_write_tokens_to_workflows": false,
            "send_secrets_and_variables": false,
            "require_approval_for_fork_pr_workflows": false
        }))
    };
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/permissions/fork-pr-workflows-private-repos"
        )))
        .respond_with(fork_pr_response)
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/permissions/access"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"access_level": "organization"})),
        )
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/oidc/customization/sub"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "use_default": true,
            "include_claim_keys": ["repo", "ref"]
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/actions/workflows")))
        .respond_with(ResponseTemplate::new(200).set_body_json(overrides.workflows))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/actions/variables")))
        .respond_with(ResponseTemplate::new(200).set_body_json(overrides.variables))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/actions/secrets")))
        .respond_with(ResponseTemplate::new(200).set_body_json(overrides.secrets))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/organization-secrets"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(overrides.organization_secrets))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/actions/organization-variables"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(overrides.organization_variables))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/dependabot/secrets")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"secrets": []})))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/codespaces/secrets")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"secrets": []})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn collect_reports_current_settings_matching_baseline() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    let settings = collected.category.settings.unwrap();
    assert_eq!(settings.enabled, Some(true));
    assert_eq!(settings.allowed_actions.as_deref(), Some("all"));
    assert_eq!(settings.artifact_retention_days, Some(90));
    assert_eq!(settings.log_retention_days, Some(90));
    assert_eq!(settings.cache_retention_limit_days, Some(7));
    assert_eq!(settings.cache_storage_limit_gb, Some(10));
    assert_eq!(
        settings.fork_pull_request_contributor_approval.as_deref(),
        Some("first_time_contributors")
    );
    assert_eq!(
        settings.workflow_access_level.as_deref(),
        Some("organization")
    );
}

#[tokio::test]
async fn plan_is_empty_when_desired_matches_baseline_idempotence() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            enabled: Some(true),
            allowed_actions: Some("all".to_owned()),
            artifact_retention_days: Some(90),
            fork_pull_request_contributor_approval: Some("first_time_contributors".to_owned()),
            default_workflow_permissions: Some("read".to_owned()),
            can_approve_pull_request_reviews: Some(false),
            workflow_access_level: Some("organization".to_owned()),
            private_fork_workflows_enabled: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        !plan.has_actionable_changes(),
        "expected no-op plan, got: {plan:?}"
    );
    assert!(plan.issues.is_empty());
}

#[tokio::test]
async fn plan_detects_permissions_drift_and_apply_applies_it() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/actions/permissions"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            enabled: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };

    let client = client(&server);
    let collected = collect_actions_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);
    assert!(plan.has_actionable_changes());
    assert!(matches!(
        plan.settings_changes.first(),
        Some(ActionsSettingChange::Permissions { enabled: false, .. })
    ));

    let result = apply_actions_plan(&client, "my-repo", &plan).await.unwrap();
    assert!(result.issues.is_empty());
    assert!(!result.applied.is_empty());
}

#[tokio::test]
async fn conflicting_artifact_and_log_retention_is_a_blocker() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            artifact_retention_days: Some(30),
            log_retention_days: Some(60),
            ..Default::default()
        }),
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(plan.issues.iter().any(|issue| issue.scope
        == "actions.settings.artifact_retention_days"
        && issue.severity == IssueSeverity::Blocker));
}

#[tokio::test]
async fn private_fork_pr_workflow_policy_not_applicable_on_public_repo() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "public-repo",
        BaselineOverrides {
            fork_pr_workflows_status: 404,
            ..Default::default()
        },
    )
    .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            private_fork_workflows_enabled: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "public-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    // Should surface as a non-blocking warning, not attempt a write.
    assert!(
        !plan
            .settings_changes
            .iter()
            .any(|change| matches!(change, ActionsSettingChange::PrivateForkPrWorkflows { .. }))
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Warning)
    );
}

#[tokio::test]
async fn workflow_state_change_is_idempotent_when_already_matching() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            workflows: json!({
                "workflows": [
                    {"id": 1, "path": ".github/workflows/ci.yml", "state": "active"}
                ]
            }),
            ..Default::default()
        },
    )
    .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        workflows: vec![WorkflowStateConfig {
            path: ".github/workflows/ci.yml".to_owned(),
            enabled: Some(true),
        }],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(plan.workflow_state_changes.is_empty());
}

#[tokio::test]
async fn disabled_workflow_drift_produces_a_state_change() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            workflows: json!({
                "workflows": [
                    {"id": 7, "path": ".github/workflows/deploy.yml", "state": "disabled_manually"}
                ]
            }),
            ..Default::default()
        },
    )
    .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        workflows: vec![WorkflowStateConfig {
            path: ".github/workflows/deploy.yml".to_owned(),
            enabled: Some(true),
        }],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert_eq!(plan.workflow_state_changes.len(), 1);
    assert_eq!(
        plan.workflow_state_changes[0].path,
        ".github/workflows/deploy.yml"
    );
    assert!(plan.workflow_state_changes[0].enabled);
}

#[tokio::test]
async fn variables_are_diffed_by_value_and_are_idempotent() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            variables: json!({"variables": [{"name": "REGION", "value": "eu-west-1"}]}),
            ..Default::default()
        },
    )
    .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        variables: vec![NamedValueConfig {
            name: "REGION".to_owned(),
            value: "eu-west-1".to_owned(),
        }],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);
    assert!(
        plan.variable_upserts.is_empty(),
        "expected no-op, matching value"
    );

    // Changing the value must produce an upsert.
    let desired_changed = ActionsCategoryV2 {
        variables: vec![NamedValueConfig {
            name: "REGION".to_owned(),
            value: "us-east-1".to_owned(),
        }],
        ..desired
    };
    let plan_changed = plan_actions_category(&desired_changed, &collected);
    assert_eq!(plan_changed.variable_upserts.len(), 1);
}

#[tokio::test]
async fn secret_with_unresolved_manual_placeholder_is_blocked() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        secrets: vec![SecretPlaceholderConfig {
            name: "DEPLOY_TOKEN".to_owned(),
            value_from: ExternalValueReference::Manual { hint: None },
        }],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(plan.secret_upserts.is_empty());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.scope == "actions.secrets.DEPLOY_TOKEN"
                && issue.severity == IssueSeverity::Blocker)
    );
}

#[tokio::test]
async fn secret_resolved_from_env_is_encrypted_and_never_logs_plaintext() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    // SAFETY: test-local; no other thread in this test reads/writes this key.
    unsafe {
        std::env::set_var("WARD_TEST_DEPLOY_TOKEN", "plaintext-secret-value");
    }

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        secrets: vec![SecretPlaceholderConfig {
            name: "DEPLOY_TOKEN".to_owned(),
            value_from: ExternalValueReference::Env {
                key: "WARD_TEST_DEPLOY_TOKEN".to_owned(),
            },
        }],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert_eq!(plan.secret_upserts.len(), 1);
    let resolved = &plan.secret_upserts[0];
    assert_eq!(resolved.name, "DEPLOY_TOKEN");
    assert_eq!(
        resolved.value.expose_for_encryption(),
        "plaintext-secret-value"
    );

    // Debug output must always redact, regardless of what struct wraps it.
    let debug_output = format!("{resolved:?}");
    assert!(!debug_output.contains("plaintext-secret-value"));
    assert!(debug_output.contains("REDACTED"));

    unsafe {
        std::env::remove_var("WARD_TEST_DEPLOY_TOKEN");
    }
}

#[tokio::test]
async fn apply_encrypts_secret_with_target_public_key_before_put() {
    use base64::Engine;
    use crypto_box::SecretKey;
    use crypto_box::aead::OsRng;

    let secret_key = SecretKey::generate(&mut OsRng);
    let public_key_b64 =
        base64::engine::general_purpose::STANDARD.encode(secret_key.public_key().as_bytes());

    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/actions/secrets/public-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "key_id": "key-1",
            "key": public_key_b64
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/actions/secrets/DEPLOY_TOKEN"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("WARD_TEST_APPLY_TOKEN", "another-plaintext-value");
    }

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        secrets: vec![SecretPlaceholderConfig {
            name: "DEPLOY_TOKEN".to_owned(),
            value_from: ExternalValueReference::Env {
                key: "WARD_TEST_APPLY_TOKEN".to_owned(),
            },
        }],
        ..Default::default()
    };

    let client = client(&server);
    let collected = collect_actions_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);
    let result = apply_actions_plan(&client, "my-repo", &plan).await.unwrap();

    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );
    assert!(
        result
            .applied
            .iter()
            .any(|scope| scope.contains("DEPLOY_TOKEN"))
    );

    unsafe {
        std::env::remove_var("WARD_TEST_APPLY_TOKEN");
    }
}

#[tokio::test]
async fn secret_pruning_deletes_remote_only_secrets_when_prune_enabled() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            secrets: json!({"secrets": [{"name": "STALE_SECRET"}]}),
            ..Default::default()
        },
    )
    .await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/my-repo/actions/secrets/STALE_SECRET"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(true),
        secrets: vec![],
        ..Default::default()
    };

    let client = client(&server);
    let collected = collect_actions_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);
    assert_eq!(plan.secret_deletions, vec!["STALE_SECRET".to_owned()]);

    let result = apply_actions_plan(&client, "my-repo", &plan).await.unwrap();
    assert!(result.issues.is_empty());
}

#[tokio::test]
async fn observe_disposition_never_produces_changes() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let desired = ActionsCategoryV2 {
        policy: CategoryPolicy::observe(),
        settings: Some(ActionsSettingsConfig {
            enabled: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);
    assert!(!plan.has_actionable_changes());
}

#[tokio::test]
async fn verify_reports_compliant_when_state_matches_desired() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            enabled: Some(true),
            allowed_actions: Some("all".to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = verify_actions_category(&client(&server), "my-repo", &desired)
        .await
        .unwrap();
    assert!(
        result.compliant,
        "expected compliant, plan: {:?}",
        result.plan
    );
}

// ---------------------------------------------------------------------------
// Hardening: classified reads must not abort collection (issues #1, #2, #4).
// ---------------------------------------------------------------------------

/// Live private repositories respond `422 Unprocessable Entity` to
/// `fork-pr-contributor-approval` with a message like "Fork PR approval is
/// not allowed for private repositories". This must be classified as
/// `NotApplicable`, not propagate as an `Err`.
#[tokio::test]
async fn fork_pr_contributor_approval_422_on_private_repo_is_not_applicable() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "private-repo", BaselineOverrides::default()).await;
    // Override with a 422 (ties resolve to the last-registered mock in
    // wiremock only when priorities differ; here we mount a strictly higher
    // priority mock to guarantee this response wins over the baseline 200).
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/private-repo/actions/permissions/fork-pr-contributor-approval",
        ))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Fork PR approval is not allowed for private repositories"
        })))
        .with_priority(1)
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "private-repo", None)
        .await
        .expect("422 on an optional endpoint must not abort collection");

    assert_eq!(
        collected
            .category
            .settings
            .unwrap()
            .fork_pull_request_contributor_approval,
        None
    );
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "actions/permissions/fork-pr-contributor-approval"
            && entry.outcome == CoverageOutcome::NotApplicable
    }));
}

/// A 403 on one optional endpoint must be recorded as `PermissionDenied`
/// coverage and must not prevent the rest of the category from collecting.
#[tokio::test]
async fn permission_denied_on_one_endpoint_still_collects_the_rest() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/actions/permissions/workflow"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Must have admin rights to Repository."
        })))
        .with_priority(1)
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .expect("403 on one endpoint must not abort collection");

    let settings = collected.category.settings.unwrap();
    // The permission-denied field is absent...
    assert_eq!(settings.default_workflow_permissions, None);
    // ...but everything else collected normally.
    assert_eq!(settings.enabled, Some(true));
    assert_eq!(settings.artifact_retention_days, Some(90));
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "actions/permissions/workflow"
            && entry.outcome == CoverageOutcome::PermissionDenied
            && entry.category == ManifestCategoryName::Actions
    }));
}

/// With `desired = None` (a source-import snapshot), every workflow's
/// enabled state must be collected, not just ones named by a desired config.
#[tokio::test]
async fn import_snapshot_collects_all_workflow_states() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            workflows: json!({
                "workflows": [
                    {"id": 1, "path": ".github/workflows/ci.yml", "state": "active"},
                    {"id": 2, "path": ".github/workflows/deploy.yml", "state": "disabled_manually"},
                    {"id": 3, "path": ".github/workflows/nightly.yml", "state": "active"}
                ]
            }),
            ..Default::default()
        },
    )
    .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    assert_eq!(collected.category.workflows.len(), 3);
    assert!(
        collected
            .category
            .workflows
            .iter()
            .any(|w| w.path == ".github/workflows/ci.yml" && w.enabled == Some(true))
    );
    assert!(
        collected
            .category
            .workflows
            .iter()
            .any(|w| w.path == ".github/workflows/deploy.yml" && w.enabled == Some(false))
    );
    assert!(
        collected
            .category
            .workflows
            .iter()
            .any(|w| w.path == ".github/workflows/nightly.yml" && w.enabled == Some(true))
    );
}

/// Dependabot/Codespaces secret *names* must be preserved as manifest
/// placeholders (via `dependabot_secrets`/`codespaces_secrets`), not merely
/// as coverage — a snapshot must not collapse them into a count string.
#[tokio::test]
async fn dependabot_and_codespaces_secret_names_are_preserved_as_placeholders() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/dependabot/secrets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "secrets": [{"name": "PRIMARY_TOKEN"}, {"name": "SECONDARY_TOKEN"}]
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/codespaces/secrets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "secrets": [{"name": "DEV_CONTAINER_TOKEN"}]
        })))
        .with_priority(1)
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    assert!(
        collected
            .coverage
            .iter()
            .any(|entry| entry.endpoint == "dependabot/secrets/PRIMARY_TOKEN"
                && entry.outcome == CoverageOutcome::Collected)
    );
    assert!(collected.coverage.iter().any(|entry| entry.endpoint
        == "dependabot/secrets/SECONDARY_TOKEN"
        && entry.outcome == CoverageOutcome::Collected));
    assert!(collected.coverage.iter().any(|entry| entry.endpoint
        == "codespaces/secrets/DEV_CONTAINER_TOKEN"
        && entry.outcome == CoverageOutcome::Collected));

    let dependabot_names: Vec<&str> = collected
        .category
        .dependabot_secrets
        .iter()
        .map(|placeholder| placeholder.name.as_str())
        .collect();
    assert_eq!(dependabot_names, vec!["PRIMARY_TOKEN", "SECONDARY_TOKEN"]);
    let codespaces_names: Vec<&str> = collected
        .category
        .codespaces_secrets
        .iter()
        .map(|placeholder| placeholder.name.as_str())
        .collect();
    assert_eq!(codespaces_names, vec!["DEV_CONTAINER_TOKEN"]);
}

// ---------------------------------------------------------------------------
// Hardening: private-fork-workflow policy must round-trip all four fields
// (issue #3), instead of resetting three of them to `false` on every write.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn private_fork_pr_workflow_apply_preserves_other_three_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/actions/permissions/fork-pr-workflows-private-repos",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "run_workflows_from_fork_pull_requests": false,
            "send_write_tokens_to_workflows": true,
            "send_secrets_and_variables": true,
            "require_approval_for_fork_pr_workflows": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/repos/test-org/my-repo/actions/permissions/fork-pr-workflows-private-repos",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let plan = ActionsPlan {
        settings_changes: vec![ActionsSettingChange::PrivateForkPrWorkflows {
            run_workflows_from_fork_pull_requests: true,
            send_write_tokens_to_workflows: None,
            send_secrets_and_variables: None,
            require_approval_for_fork_pr_workflows: None,
        }],
        workflow_state_changes: Vec::new(),
        variable_upserts: Vec::new(),
        variable_deletions: Vec::new(),
        secret_upserts: Vec::new(),
        secret_deletions: Vec::new(),
        reference_actions: Vec::new(),
        issues: Vec::new(),
    };

    let client = client(&server);
    let result = apply_actions_plan(&client, "my-repo", &plan).await.unwrap();
    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );

    let requests = server.received_requests().await.unwrap();
    let put = requests
        .iter()
        .find(|request| request.method.as_str() == "PUT")
        .expect("expected a PUT request");
    let body: Value = serde_json::from_slice(&put.body).unwrap();
    assert_eq!(body["run_workflows_from_fork_pull_requests"], true);
    // The other three fields must be preserved from the live read, not reset.
    assert_eq!(body["send_write_tokens_to_workflows"], true);
    assert_eq!(body["send_secrets_and_variables"], true);
    assert_eq!(body["require_approval_for_fork_pr_workflows"], true);
}

/// All four combinations of the two manifest booleans that both map onto
/// `run_workflows_from_fork_pull_requests` must round-trip through plan
/// (agreeing values only; conflicting values are already covered by other
/// tests as a blocker).
#[tokio::test]
async fn private_fork_pr_workflow_policy_round_trips_all_boolean_values() {
    for desired_value in [true, false] {
        let server = MockServer::start().await;
        mount_actions_baseline(
            &server,
            "my-repo",
            BaselineOverrides {
                fork_pr_workflows_status: 200,
                ..Default::default()
            },
        )
        .await;

        let desired = ActionsCategoryV2 {
            policy: managed_policy(false),
            settings: Some(ActionsSettingsConfig {
                private_fork_workflows_enabled: Some(desired_value),
                fork_pull_request_workflows_enabled: Some(desired_value),
                ..Default::default()
            }),
            ..Default::default()
        };

        let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
            .await
            .unwrap();
        let plan = plan_actions_category(&desired, &collected);

        // Baseline mounts `run_workflows_from_fork_pull_requests: true`, so a
        // change is only planned when `desired_value` disagrees with it.
        let expects_change = !desired_value;
        assert_eq!(
            plan.settings_changes.iter().any(|change| matches!(
                change,
                ActionsSettingChange::PrivateForkPrWorkflows {
                    run_workflows_from_fork_pull_requests,
                    ..
                } if *run_workflows_from_fork_pull_requests == desired_value
            )),
            expects_change,
            "desired_value={desired_value}, plan={plan:?}"
        );
    }
}

/// When the manifest explicitly sets one of the three sibling booleans
/// (`send_write_tokens_to_workflows`, `send_secrets_and_variables`,
/// `require_approval_for_fork_pr_workflows`), that value must be planned
/// and applied as an override — not silently discarded in favor of the
/// live value — while any unspecified sibling booleans are still preserved
/// from the live read.
#[tokio::test]
async fn private_fork_pr_workflow_explicit_override_is_applied_and_others_preserved() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            fork_pr_workflows_status: 200,
            ..Default::default()
        },
    )
    .await;
    // Baseline mounts run=true, write_tokens=false, secrets_vars=false,
    // approval=false. Only override `send_write_tokens_to_workflows`.
    Mock::given(method("PUT"))
        .and(path(
            "/repos/test-org/my-repo/actions/permissions/fork-pr-workflows-private-repos",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            send_write_tokens_to_workflows: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        plan.settings_changes
            .iter()
            .any(|change| matches!(change, ActionsSettingChange::PrivateForkPrWorkflows { .. })),
        "expected an explicit override of send_write_tokens_to_workflows to be planned, plan={plan:?}"
    );

    let result = apply_actions_plan(&client(&server), "my-repo", &plan)
        .await
        .unwrap();
    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );

    let requests = server.received_requests().await.unwrap();
    let put = requests
        .iter()
        .find(|request| request.method.as_str() == "PUT")
        .expect("expected a PUT request");
    let body: Value = serde_json::from_slice(&put.body).unwrap();
    // Run-workflows is unspecified in the manifest, so it must be preserved
    // from the live baseline (`true`), not reset.
    assert_eq!(body["run_workflows_from_fork_pull_requests"], true);
    // The explicit override must be applied.
    assert_eq!(body["send_write_tokens_to_workflows"], true);
    // The other two unspecified booleans must be preserved from the live
    // read (`false`), not reset or coerced to the override's value.
    assert_eq!(body["send_secrets_and_variables"], false);
    assert_eq!(body["require_approval_for_fork_pr_workflows"], false);
}

/// Dependabot/Codespaces secret placeholders in the manifest are not
/// silently dropped even though write support for those two secret
/// families isn't implemented yet: `plan_actions_category` must surface a
/// warning naming every desired secret so drift isn't hidden.
#[tokio::test]
async fn dependabot_and_codespaces_secret_placeholders_surface_a_warning_when_not_applied() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        dependabot_secrets: vec![SecretPlaceholderConfig {
            name: "DEPENDABOT_TOKEN".to_owned(),
            value_from: ExternalValueReference::Manual { hint: None },
        }],
        codespaces_secrets: vec![SecretPlaceholderConfig {
            name: "CODESPACES_TOKEN".to_owned(),
            value_from: ExternalValueReference::Manual { hint: None },
        }],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Warning
                && issue.message.contains("DEPENDABOT_TOKEN")),
        "expected a warning naming the unapplied Dependabot secret, issues={:?}",
        plan.issues
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Warning
                && issue.message.contains("CODESPACES_TOKEN")),
        "expected a warning naming the unapplied Codespaces secret, issues={:?}",
        plan.issues
    );
    // No secret writes are attempted for these two families.
    assert!(
        plan.secret_upserts
            .iter()
            .all(|secret| secret.name != "DEPENDABOT_TOKEN" && secret.name != "CODESPACES_TOKEN")
    );
}

// ---------------------------------------------------------------------------
// Hardening: secret idempotence by name (issue #6).
// ---------------------------------------------------------------------------

/// If a secret name is already present remotely, it must not be re-resolved
/// or re-planned — even when the external value is unresolvable — and
/// `verify_actions_category` must report compliant.
#[tokio::test]
async fn already_present_secret_is_not_replanned_and_verify_converges() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            secrets: json!({"secrets": [{"name": "DEPLOY_TOKEN"}]}),
            ..Default::default()
        },
    )
    .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        secrets: vec![SecretPlaceholderConfig {
            name: "DEPLOY_TOKEN".to_owned(),
            // Deliberately unresolvable: no env var set, no hint.
            value_from: ExternalValueReference::Manual { hint: None },
        }],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        plan.secret_upserts.is_empty(),
        "an already-present secret must not be resolved/upserted: {plan:?}"
    );
    assert!(
        !plan
            .issues
            .iter()
            .any(|issue| issue.scope == "actions.secrets.DEPLOY_TOKEN"),
        "an already-present secret must not produce an unresolved-value blocker: {:?}",
        plan.issues
    );

    let result = verify_actions_category(&client(&server), "my-repo", &desired)
        .await
        .unwrap();
    assert!(
        result.compliant,
        "verify must converge once the secret already exists: {:?}",
        result.plan
    );
}

/// A genuinely missing secret must still be resolved/upserted even when
/// another secret already exists remotely (idempotence must be per-name).
#[tokio::test]
async fn missing_secret_is_still_resolved_when_another_secret_already_exists() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            secrets: json!({"secrets": [{"name": "EXISTING_TOKEN"}]}),
            ..Default::default()
        },
    )
    .await;

    unsafe {
        std::env::set_var("WARD_TEST_NEW_TOKEN", "brand-new-value");
    }

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        secrets: vec![
            SecretPlaceholderConfig {
                name: "EXISTING_TOKEN".to_owned(),
                value_from: ExternalValueReference::Manual { hint: None },
            },
            SecretPlaceholderConfig {
                name: "NEW_TOKEN".to_owned(),
                value_from: ExternalValueReference::Env {
                    key: "WARD_TEST_NEW_TOKEN".to_owned(),
                },
            },
        ],
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert_eq!(plan.secret_upserts.len(), 1);
    assert_eq!(plan.secret_upserts[0].name, "NEW_TOKEN");

    unsafe {
        std::env::remove_var("WARD_TEST_NEW_TOKEN");
    }
}

// ---------------------------------------------------------------------------
// Coverage follow-up: self-hosted runners (observed references) and runner
// groups (documented repo-endpoint limitation).
// ---------------------------------------------------------------------------

/// Self-hosted runners observed on the repository must be collected as
/// read-only `Runner` references with a stable, human-readable compact name
/// (name + status + sorted labels) — never the numeric `id`/`runner_group_id`
/// GitHub returns, which are internal source identifiers.
#[tokio::test]
async fn observed_runners_are_recorded_as_references_without_source_ids() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/actions/runners"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 2,
            "runners": [
                {
                    "id": 12345,
                    "runner_group_id": 9,
                    "name": "self-hosted-1",
                    "os": "linux",
                    "status": "online",
                    "busy": false,
                    "labels": [
                        {"id": 1, "name": "self-hosted", "type": "read-only"},
                        {"id": 2, "name": "linux", "type": "read-only"},
                        {"id": 3, "name": "gpu", "type": "custom"}
                    ]
                },
                {
                    "id": 67890,
                    "runner_group_id": 9,
                    "name": "self-hosted-2",
                    "os": "linux",
                    "status": "offline",
                    "busy": false,
                    "labels": []
                }
            ]
        })))
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    let runner_refs: Vec<_> = collected
        .category
        .references
        .iter()
        .filter(|reference| reference.resource_type == ReferencedResourceType::Runner)
        .collect();
    assert_eq!(runner_refs.len(), 2);

    let names: Vec<&str> = runner_refs.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"self-hosted-1 [status=online, labels=gpu,linux,self-hosted]"));
    assert!(names.contains(&"self-hosted-2 [status=offline, labels=]"));

    // The numeric runner/runner-group ids must never leak into the stored
    // diagnostic name.
    for name in &names {
        assert!(!name.contains("12345"));
        assert!(!name.contains("67890"));
        assert!(!name.contains('9'), "must not leak runner_group_id: {name}");
    }
}

/// Runner groups have no documented repository-scoped endpoint (only the
/// organization-scoped `GET /orgs/{org}/actions/runner-groups` supports
/// `visible_to_repository`, requiring org-admin scope). This must always be
/// recorded as an explicit `Unsupported` coverage entry rather than silently
/// omitted, whether or not runners themselves were observed successfully.
#[tokio::test]
async fn runner_group_visibility_is_recorded_as_an_unsupported_coverage_entry() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/actions/runners"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"total_count": 0, "runners": []})),
        )
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    let entry = collected
        .coverage
        .iter()
        .find(|entry| entry.endpoint == "actions/runner-groups")
        .expect("expected an explicit runner-groups coverage entry");
    assert_eq!(entry.category, ManifestCategoryName::Actions);
    assert_eq!(entry.outcome, CoverageOutcome::Unsupported);
    assert!(
        entry
            .reason
            .as_ref()
            .unwrap()
            .contains("visible_to_repository")
    );
}

// ---------------------------------------------------------------------------
// Organization secret/variable reference resolution and association
// ---------------------------------------------------------------------------
//
// Ward never alters an organization secret/variable's own value or
// visibility here: only the per-repository `selected` association is ever
// written, and only when the Actions category is explicitly `managed` (the
// baseline for every test below) *and* `sensitive`. Permission failures
// while resolving are always surfaced as blockers, never treated as "not
// associated" or silently satisfied.

fn desired_with_org_secret_reference(policy: CategoryPolicy, name: &str) -> ActionsCategoryV2 {
    ActionsCategoryV2 {
        policy,
        references: vec![ReferencedResourceConfig {
            resource_type: ReferencedResourceType::OrganizationSecret,
            name: name.to_owned(),
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn organization_secret_all_visibility_is_satisfied_without_association_call() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/secrets/DEPLOYKEY"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"name": "DEPLOYKEY", "visibility": "all"})),
        )
        .mount(&server)
        .await;

    let desired = desired_with_org_secret_reference(sensitive_managed_policy(false), "DEPLOYKEY");
    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        !plan.has_actionable_changes(),
        "all-visibility secret needs no association: {plan:?}"
    );
    assert!(
        plan.issues.is_empty(),
        "unexpected issues: {:?}",
        plan.issues
    );

    // Never touches the selected-repositories or association sub-endpoints
    // for an `all`-visibility secret.
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|request| !request.url.path().contains("/repositories")),
        "must not call selected-repository endpoints for all-visibility secrets"
    );
}

#[tokio::test]
async fn organization_secret_selected_and_associated_is_satisfied() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/secrets/DEPLOYKEY"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "DEPLOYKEY",
            "visibility": "selected"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/orgs/test-org/actions/secrets/DEPLOYKEY/repositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 1,
            "repositories": [{"id": 1, "name": "my-repo"}]
        })))
        .mount(&server)
        .await;

    let desired = desired_with_org_secret_reference(sensitive_managed_policy(false), "DEPLOYKEY");
    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        !plan.has_actionable_changes(),
        "already-associated secret needs no action: {plan:?}"
    );
    assert!(
        plan.issues.is_empty(),
        "unexpected issues: {:?}",
        plan.issues
    );
}

#[tokio::test]
async fn organization_secret_selected_not_associated_proposes_and_applies_association_when_sensitive()
 {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/secrets/DEPLOYKEY"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "DEPLOYKEY",
            "visibility": "selected"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/orgs/test-org/actions/secrets/DEPLOYKEY/repositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 0,
            "repositories": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 42})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/orgs/test-org/actions/secrets/DEPLOYKEY/repositories/42",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let desired = desired_with_org_secret_reference(sensitive_managed_policy(false), "DEPLOYKEY");
    let client = client(&server);
    let collected = collect_actions_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert_eq!(
        plan.reference_actions,
        vec![OrgReferenceAction::Associate(ReferencedResourceConfig {
            resource_type: ReferencedResourceType::OrganizationSecret,
            name: "DEPLOYKEY".to_owned(),
        })]
    );

    let result = apply_actions_plan(&client, "my-repo", &plan).await.unwrap();
    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.method.as_str() == "PUT"
                && request.url.path()
                    == "/orgs/test-org/actions/secrets/DEPLOYKEY/repositories/42"),
        "expected the per-repository association PUT to be sent"
    );
    // Never touches the secret's own value/visibility.
    assert!(
        requests.iter().all(|request| request.url.path()
            != "/orgs/test-org/actions/secrets/DEPLOYKEY"
            || request.method.as_str() == "GET"),
        "must never write to the organization secret's own value/visibility endpoint"
    );
}

#[tokio::test]
async fn organization_secret_selected_not_associated_is_blocked_without_sensitive_and_not_applied()
{
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/secrets/DEPLOYKEY"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "DEPLOYKEY",
            "visibility": "selected"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/orgs/test-org/actions/secrets/DEPLOYKEY/repositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 0,
            "repositories": []
        })))
        .mount(&server)
        .await;

    // `managed_policy` (not sensitive): association must not be planned.
    let desired = desired_with_org_secret_reference(managed_policy(false), "DEPLOYKEY");
    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        plan.reference_actions.is_empty(),
        "association must require policy.sensitive: {plan:?}"
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker
                && issue.message.contains("sensitive")),
        "expected a blocker requiring policy.sensitive: {:?}",
        plan.issues
    );

    // No PUT/DELETE was ever sent to the association endpoint.
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|request| !request.url.path().contains("/repositories/")),
        "must not call the per-repository association endpoint without sensitive"
    );
}

#[tokio::test]
async fn organization_secret_metadata_permission_denied_is_a_blocker_not_assumed_absence() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/secrets/DEPLOYKEY"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "Forbidden"})))
        .mount(&server)
        .await;

    let desired = desired_with_org_secret_reference(sensitive_managed_policy(false), "DEPLOYKEY");
    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        plan.reference_actions.is_empty(),
        "permission-denied lookup must never produce an association action: {plan:?}"
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker),
        "permission denial must be a blocker, not assumed absence: {:?}",
        plan.issues
    );
    let coverage_entry = collected
        .coverage
        .iter()
        .find(|entry| entry.endpoint == "actions/organization-secrets/metadata")
        .expect("expected a coverage entry for the permission-denied metadata lookup");
    assert_eq!(coverage_entry.outcome, CoverageOutcome::PermissionDenied);
}

#[tokio::test]
async fn organization_secret_selected_repositories_permission_denied_keeps_reference_blocked() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/secrets/DEPLOYKEY"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "DEPLOYKEY",
            "visibility": "selected"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/orgs/test-org/actions/secrets/DEPLOYKEY/repositories",
        ))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "Forbidden"})))
        .mount(&server)
        .await;

    let desired = desired_with_org_secret_reference(sensitive_managed_policy(false), "DEPLOYKEY");
    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    // Must NOT be silently treated as satisfied and must NOT propose an
    // association action (we genuinely do not know current state).
    assert!(
        plan.reference_actions.is_empty(),
        "unknown association state must never produce an action: {plan:?}"
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker
                && issue.message.contains("org-admin scope")),
        "expected a blocker stating the exact limitation: {:?}",
        plan.issues
    );
}

#[tokio::test]
async fn organization_secret_not_found_in_target_org_is_a_blocker() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/secrets/MISSINGSECRET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&server)
        .await;

    let desired =
        desired_with_org_secret_reference(sensitive_managed_policy(false), "MISSINGSECRET");
    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(plan.reference_actions.is_empty());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker
                && issue.message.contains("does not exist")),
        "expected a blocker for a referenced secret absent from the target org: {:?}",
        plan.issues
    );
}

#[tokio::test]
async fn organization_variable_selected_not_associated_and_prune_disassociates_when_sensitive() {
    let server = MockServer::start().await;
    // The repo currently has visibility into `LEGACYVAR` (via the
    // repo-scoped "visible to this repo" listing), but it is no longer in
    // `desired.references`, so pruning should disassociate it — only
    // because the org variable's own visibility is `selected` (a reversible
    // per-repository association) and both `prune` and `sensitive` are set.
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            organization_variables: json!({"variables": [{"name": "LEGACYVAR", "value": "irrelevant"}]}),
            ..BaselineOverrides::default()
        },
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/orgs/test-org/actions/variables/LEGACYVAR"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "LEGACYVAR",
            "visibility": "selected"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/orgs/test-org/actions/variables/LEGACYVAR/repositories",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total_count": 1,
            "repositories": [{"id": 7, "name": "my-repo"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 7})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/orgs/test-org/actions/variables/LEGACYVAR/repositories/7",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    // Nothing desired references `LEGACYVAR`; prune + sensitive is set.
    let desired = ActionsCategoryV2 {
        policy: sensitive_managed_policy(true),
        ..Default::default()
    };
    let client = client(&server);
    let collected = collect_actions_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert_eq!(
        plan.reference_actions,
        vec![OrgReferenceAction::Disassociate(ReferencedResourceConfig {
            resource_type: ReferencedResourceType::OrganizationVariable,
            name: "LEGACYVAR".to_owned(),
        })]
    );

    let result = apply_actions_plan(&client, "my-repo", &plan).await.unwrap();
    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.method.as_str() == "DELETE"
                && request.url.path()
                    == "/orgs/test-org/actions/variables/LEGACYVAR/repositories/7"),
        "expected the per-repository disassociation DELETE to be sent"
    );
}

#[tokio::test]
async fn organization_variable_visible_but_not_desired_without_prune_is_left_alone() {
    mount_actions_baseline_with_org_variable_visible().await;
}

async fn mount_actions_baseline_with_org_variable_visible() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            organization_variables: json!({"variables": [{"name": "LEGACYVAR", "value": "irrelevant"}]}),
            ..BaselineOverrides::default()
        },
    )
    .await;

    // No `prune`: the currently-visible-but-undesired variable must not be
    // resolved or disassociated at all (no org-scoped metadata call).
    let desired = ActionsCategoryV2 {
        policy: sensitive_managed_policy(false),
        ..Default::default()
    };
    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        plan.reference_actions.is_empty(),
        "must never disassociate without prune: {plan:?}"
    );
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|request| !request.url.path().starts_with("/orgs/")),
        "must not resolve org-scoped metadata for undesired references without prune"
    );
}

// ---------------------------------------------------------------------------
// Actions cache retention-limit / storage-limit
// (`/repos/{owner}/{repo}/actions/cache/retention-limit` and
// `.../storage-limit`). Cache *usage* (`.../actions/cache/usage`) is runtime
// data, never collected or planned by this category.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cache_limits_are_collected_from_baseline() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .unwrap();

    let settings = collected.category.settings.unwrap();
    assert_eq!(settings.cache_retention_limit_days, Some(7));
    assert_eq!(settings.cache_storage_limit_gb, Some(10));
}

#[tokio::test]
async fn cache_limits_plan_is_empty_when_desired_matches_observed_idempotence() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            cache_retention_limit_days: Some(7),
            cache_storage_limit_gb: Some(10),
            ..Default::default()
        }),
        ..Default::default()
    };

    let collected = collect_actions_category(&client(&server), "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);

    assert!(
        !plan.has_actionable_changes(),
        "matching cache limits must be a no-op plan, got: {plan:?}"
    );
    assert!(plan.issues.is_empty());
}

#[tokio::test]
async fn cache_limits_drift_is_planned_and_applied() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("PUT"))
        .and(path(
            "/repos/test-org/my-repo/actions/cache/retention-limit",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/actions/cache/storage-limit"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            cache_retention_limit_days: Some(3),
            cache_storage_limit_gb: Some(5),
            ..Default::default()
        }),
        ..Default::default()
    };

    let client = client(&server);
    let collected = collect_actions_category(&client, "my-repo", Some(&desired))
        .await
        .unwrap();
    let plan = plan_actions_category(&desired, &collected);
    assert!(plan.has_actionable_changes());
    assert!(plan.settings_changes.iter().any(|change| matches!(
        change,
        ActionsSettingChange::CacheRetentionLimit {
            max_cache_retention_days: 3
        }
    )));
    assert!(plan.settings_changes.iter().any(|change| matches!(
        change,
        ActionsSettingChange::CacheStorageLimit {
            max_cache_size_gb: 5
        }
    )));

    let result = apply_actions_plan(&client, "my-repo", &plan).await.unwrap();
    assert!(
        result.issues.is_empty(),
        "unexpected issues: {:?}",
        result.issues
    );
    assert!(!result.applied.is_empty());

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.method.as_str() == "PUT"
                && request.url.path() == "/repos/test-org/my-repo/actions/cache/retention-limit"
                && request.body_json::<Value>().unwrap() == json!({"max_cache_retention_days": 3}))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.method.as_str() == "PUT"
                && request.url.path() == "/repos/test-org/my-repo/actions/cache/storage-limit"
                && request.body_json::<Value>().unwrap() == json!({"max_cache_size_gb": 5}))
    );
}

/// A `403` on the cache retention-limit endpoint must be recorded as
/// `PermissionDenied` coverage, and must not abort collection of the rest of
/// the category (mirrors the general permission-degraded-partial-collection
/// requirement for every optional endpoint).
#[tokio::test]
async fn cache_retention_limit_403_is_permission_denied_and_does_not_abort() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/actions/cache/retention-limit",
        ))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "message": "Must have admin rights to Repository."
        })))
        .with_priority(1)
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .expect("403 on cache retention-limit must not abort collection");

    let settings = collected.category.settings.unwrap();
    assert_eq!(settings.cache_retention_limit_days, None);
    // The rest of the category must still have collected successfully.
    assert_eq!(settings.cache_storage_limit_gb, Some(10));
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "actions/cache/retention-limit"
            && entry.outcome == CoverageOutcome::PermissionDenied
    }));
}

/// A `404` on the cache storage-limit endpoint (e.g. feature not enabled for
/// this plan/repository) must be recorded as coverage, not an error.
#[tokio::test]
async fn cache_storage_limit_404_is_recorded_as_coverage_and_does_not_abort() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/actions/cache/storage-limit"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .with_priority(1)
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .expect("404 on cache storage-limit must not abort collection");

    let settings = collected.category.settings.unwrap();
    assert_eq!(settings.cache_storage_limit_gb, None);
    assert_eq!(settings.cache_retention_limit_days, Some(7));
    assert!(
        collected
            .coverage
            .iter()
            .any(|entry| entry.endpoint == "actions/cache/storage-limit")
    );
}

/// A `422` on the cache retention-limit endpoint (feature unavailable for
/// this plan/repository) must classify as `NotApplicable`, not an error.
#[tokio::test]
async fn cache_retention_limit_422_is_not_applicable() {
    let server = MockServer::start().await;
    mount_actions_baseline(&server, "my-repo", BaselineOverrides::default()).await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/actions/cache/retention-limit",
        ))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Actions cache is not available for this repository"
        })))
        .with_priority(1)
        .mount(&server)
        .await;

    let collected = collect_actions_category(&client(&server), "my-repo", None)
        .await
        .expect("422 on cache retention-limit must not abort collection");

    let settings = collected.category.settings.unwrap();
    assert_eq!(settings.cache_retention_limit_days, None);
    assert!(collected.coverage.iter().any(|entry| {
        entry.endpoint == "actions/cache/retention-limit"
            && entry.outcome == CoverageOutcome::NotApplicable
    }));
}

/// Once the target already matches desired cache limits, re-planning and
/// re-verifying must be idempotent: no PUTs are sent on a second pass.
#[tokio::test]
async fn cache_limits_apply_then_verify_is_idempotent() {
    let server = MockServer::start().await;
    mount_actions_baseline(
        &server,
        "my-repo",
        BaselineOverrides {
            ..BaselineOverrides::default()
        },
    )
    .await;

    let desired = ActionsCategoryV2 {
        policy: managed_policy(false),
        settings: Some(ActionsSettingsConfig {
            cache_retention_limit_days: Some(7),
            cache_storage_limit_gb: Some(10),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = verify_actions_category(&client(&server), "my-repo", &desired)
        .await
        .unwrap();

    assert!(
        result.compliant,
        "already-matching cache limits must verify as compliant: {:?}",
        result.plan
    );
    assert!(!result.plan.has_actionable_changes());

    // No PUTs should ever have been issued for a verify-only, already
    // compliant pass.
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|request| request.method.as_str() != "PUT"),
        "verify of an already-compliant target must never write"
    );
}
