use serde_json::json;
use ward::config::manifest::{
    ActorReference, AutolinkConfigV2, CategoryPolicy, CollaboratorAccessConfig, DeployKeyConfigV2,
    ExternalValueReference, ManagementDisposition, PagesConfigV2, ReferencedResourceConfig,
    ReferencedResourceType, RepositoryAccessCategoryV2, RepositoryIntegrationsCategoryV2,
    TeamAccess, WebhookConfigV2,
};
use ward::github::Client;
use ward::reconcile::access_integrations::{
    AccessCollection, AccessPlan, AutolinkAction, CollectedAccessReference, CollectedAccessState,
    CollectedAutolink, CollectedCollaborator, CollectedDeployKey, CollectedIntegrationsState,
    CollectedPages, CollectedWebhook, DeployKeyAction, IntegrationsCollection, IntegrationsPlan,
    TeamAccessAction, apply_access, apply_integrations, canonicalize_url, plan_access,
    plan_integrations, verify_integrations_state,
};
use ward::reconcile::actions_environments::{IssueSeverity, ReconcileIssue};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn managed_sensitive_policy(prune: bool) -> CategoryPolicy {
    CategoryPolicy {
        disposition: ManagementDisposition::Managed,
        prune,
        sensitive: true,
    }
}

#[tokio::test]
async fn warning_issues_do_not_block_successful_access_or_integration_writes() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/orgs/test-org/teams/developers/repos/test-org/my-repo",
        ))
        .and(body_partial_json(json!({ "permission": "push" })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/autolinks"))
        .and(body_partial_json(json!({
            "key_prefix": "TICKET-",
            "url_template": "https://tracker.example/TICKET-<num>"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .mount(&server)
        .await;

    let warning = ReconcileIssue {
        scope: "test.warning".to_owned(),
        severity: IssueSeverity::Warning,
        message: "non-blocking".to_owned(),
    };
    let client = Client::new_for_test("test-org", &server.uri());

    let access = apply_access(
        &client,
        "my-repo",
        &AccessPlan {
            policy: managed_sensitive_policy(false),
            team_actions: vec![TeamAccessAction::Ensure(TeamAccess {
                slug: "developers".to_owned(),
                permission: "push".to_owned(),
            })],
            collaborator_actions: Vec::new(),
            reference_actions: Vec::new(),
            notes: Vec::new(),
            issues: vec![warning.clone()],
        },
    )
    .await
    .unwrap();
    assert!(access.blocked.is_empty());
    assert_eq!(access.applied.len(), 1);

    let integrations = apply_integrations(
        &client,
        "my-repo",
        &IntegrationsPlan {
            policy: managed_sensitive_policy(false),
            webhook_actions: Vec::new(),
            deploy_key_actions: Vec::new(),
            pages_action: None,
            autolink_actions: vec![AutolinkAction::Create(AutolinkConfigV2 {
                key_prefix: "TICKET-".to_owned(),
                url_template: "https://tracker.example/TICKET-<num>".to_owned(),
                is_alphanumeric: None,
            })],
            notes: Vec::new(),
            issues: vec![warning],
        },
    )
    .await
    .unwrap();
    assert!(integrations.blocked.is_empty());
    assert_eq!(integrations.applied.len(), 1);
}

#[test]
fn access_sensitive_gate_and_custom_role_reference_blocking_are_visible() {
    let current = AccessCollection {
        category: RepositoryAccessCategoryV2::default(),
        state: CollectedAccessState {
            teams: Vec::new(),
            teams_complete: true,
            collaborators: Vec::new(),
            collaborators_complete: true,
            references: vec![CollectedAccessReference {
                resource: ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::Role,
                    name: "Custom Maintainer".to_owned(),
                },
                present: Some(false),
                associated: None,
                supported: true,
                detail: None,
            }],
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let desired = RepositoryAccessCategoryV2 {
        policy: CategoryPolicy {
            disposition: ManagementDisposition::Managed,
            prune: false,
            sensitive: false,
        },
        teams: vec![TeamAccess {
            slug: "platform".to_owned(),
            permission: "Custom Maintainer".to_owned(),
        }],
        ..RepositoryAccessCategoryV2::default()
    };

    let plan = plan_access(&current, &desired);
    assert!(plan.team_actions.is_empty());
    assert!(plan.issues.iter().any(|issue| {
        issue
            .message
            .contains("Custom repository role `Custom Maintainer` is missing")
    }));
}

#[tokio::test]
async fn pending_invitation_prune_uses_invitation_delete_endpoint() {
    let current = AccessCollection {
        category: RepositoryAccessCategoryV2::default(),
        state: CollectedAccessState {
            teams: Vec::new(),
            teams_complete: true,
            collaborators: vec![CollectedCollaborator {
                config: CollaboratorAccessConfig {
                    actor: ActorReference::User {
                        login: "octocat".to_owned(),
                    },
                    permission: "push".to_owned(),
                },
                outside: false,
                pending: true,
                invitation_id: Some(42),
            }],
            collaborators_complete: true,
            references: Vec::new(),
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let desired = RepositoryAccessCategoryV2 {
        policy: managed_sensitive_policy(true),
        ..RepositoryAccessCategoryV2::default()
    };
    let plan = plan_access(&current, &desired);

    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/my-repo/invitations/42"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let report = apply_access(&client, "my-repo", &plan).await.unwrap();
    assert!(report.applied.iter().any(|line| line.contains("octocat")));
}

#[test]
fn missing_app_reference_is_blocking_but_unknown_lookup_is_not() {
    let desired = RepositoryAccessCategoryV2 {
        references: vec![ReferencedResourceConfig {
            resource_type: ReferencedResourceType::App,
            name: "deploy-protect".to_owned(),
        }],
        ..RepositoryAccessCategoryV2::default()
    };
    let missing = AccessCollection {
        category: RepositoryAccessCategoryV2::default(),
        state: CollectedAccessState {
            teams: Vec::new(),
            teams_complete: true,
            collaborators: Vec::new(),
            collaborators_complete: true,
            references: vec![CollectedAccessReference {
                resource: desired.references[0].clone(),
                present: Some(false),
                associated: None,
                supported: true,
                detail: None,
            }],
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let unknown = AccessCollection {
        state: CollectedAccessState {
            references: vec![CollectedAccessReference {
                resource: desired.references[0].clone(),
                present: None,
                associated: None,
                supported: true,
                detail: Some("lookup unavailable".to_owned()),
            }],
            ..missing.state.clone()
        },
        ..missing.clone()
    };

    let missing_plan = plan_access(&missing, &desired);
    let unknown_plan = plan_access(&unknown, &desired);
    assert!(
        missing_plan
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker)
    );
    assert!(
        unknown_plan
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Warning)
    );
}

#[tokio::test]
async fn credentialed_webhook_zero_drift_create_blocking_and_pages_blocking_work() {
    let current = IntegrationsCollection {
        category: RepositoryIntegrationsCategoryV2::default(),
        state: CollectedIntegrationsState {
            webhooks: vec![CollectedWebhook {
                id: 7,
                canonical_url: canonicalize_url("https://hooks.example.test/events"),
                config: WebhookConfigV2 {
                    url: "https://***@hooks.example.test/events".to_owned(),
                    url_from: Some(ExternalValueReference::Env {
                        key: "WARD_WEBHOOK_URL_HOOKS_EXAMPLE_TEST_EVENTS".to_owned(),
                    }),
                    active: Some(true),
                    events: vec!["push".to_owned()],
                    content_type: Some("json".to_owned()),
                    insecure_ssl: Some(false),
                    secret: Some(ExternalValueReference::Manual {
                        hint: Some("existing".to_owned()),
                    }),
                },
            }],
            webhooks_complete: true,
            deploy_keys: Vec::new(),
            deploy_keys_complete: true,
            pages: None,
            pages_complete: true,
            autolinks: Vec::new(),
            autolinks_complete: true,
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let desired_same = RepositoryIntegrationsCategoryV2 {
        policy: managed_sensitive_policy(false),
        webhooks: vec![current.state.webhooks[0].config.clone()],
        ..RepositoryIntegrationsCategoryV2::default()
    };
    assert!(plan_integrations(&current, &desired_same).is_empty());

    let desired_create = RepositoryIntegrationsCategoryV2 {
        policy: managed_sensitive_policy(false),
        webhooks: vec![WebhookConfigV2 {
            url: "https://***@hooks.example.test/new".to_owned(),
            url_from: Some(ExternalValueReference::Env {
                key: "WARD_WEBHOOK_URL_NEW".to_owned(),
            }),
            active: Some(true),
            events: vec!["push".to_owned()],
            content_type: Some("json".to_owned()),
            insecure_ssl: Some(false),
            secret: Some(ExternalValueReference::Env {
                key: "WARD_WEBHOOK_SECRET_NEW".to_owned(),
            }),
        }],
        pages: Some(PagesConfigV2 {
            build_type: Some("legacy".to_owned()),
            source_branch: Some("gh-pages".to_owned()),
            source_path: None,
            cname: None,
            https_enforced: None,
        }),
        ..RepositoryIntegrationsCategoryV2::default()
    };
    let create_plan = plan_integrations(
        &IntegrationsCollection {
            state: CollectedIntegrationsState {
                webhooks: Vec::new(),
                webhooks_complete: true,
                deploy_keys: Vec::new(),
                deploy_keys_complete: true,
                pages: None,
                pages_complete: true,
                autolinks: Vec::new(),
                autolinks_complete: true,
            },
            category: RepositoryIntegrationsCategoryV2::default(),
            coverage: Vec::new(),
            issues: Vec::new(),
        },
        &desired_create,
    );
    assert!(create_plan.webhook_actions.is_empty());
    assert!(create_plan.issues.iter().any(|issue| {
        issue.message.contains("webhook URL environment variable")
            || issue.message.contains("manual-only")
    }));
    assert!(create_plan.issues.iter().any(|issue| {
        issue
            .message
            .contains("must set both source_branch and source_path")
    }));

    let server = MockServer::start().await;
    let client = Client::new_for_test("test-org", &server.uri());
    let report = apply_integrations(&client, "my-repo", &create_plan)
        .await
        .unwrap();
    assert!(report.blocked.iter().any(|line| {
        line.contains("webhook URL environment variable WARD_WEBHOOK_URL_NEW is not set")
    }));
}

#[tokio::test]
async fn existing_webhook_update_preserves_unknown_secret() {
    let current = IntegrationsCollection {
        category: RepositoryIntegrationsCategoryV2::default(),
        state: CollectedIntegrationsState {
            webhooks: vec![CollectedWebhook {
                id: 7,
                canonical_url: canonicalize_url("https://hooks.example.test/events"),
                config: WebhookConfigV2 {
                    url: "https://***@hooks.example.test/events".to_owned(),
                    url_from: Some(ExternalValueReference::Env {
                        key: "WARD_WEBHOOK_URL_HOOKS_EXAMPLE_TEST_EVENTS".to_owned(),
                    }),
                    active: Some(true),
                    events: vec!["push".to_owned()],
                    content_type: Some("json".to_owned()),
                    insecure_ssl: Some(false),
                    secret: Some(ExternalValueReference::Manual { hint: None }),
                },
            }],
            webhooks_complete: true,
            deploy_keys: Vec::new(),
            deploy_keys_complete: true,
            pages: None,
            pages_complete: true,
            autolinks: Vec::new(),
            autolinks_complete: true,
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let desired = RepositoryIntegrationsCategoryV2 {
        policy: managed_sensitive_policy(false),
        webhooks: vec![WebhookConfigV2 {
            content_type: Some("form".to_owned()),
            ..current.state.webhooks[0].config.clone()
        }],
        ..RepositoryIntegrationsCategoryV2::default()
    };
    let plan = plan_integrations(&current, &desired);

    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/my-repo/hooks/7/config"))
        .and(body_partial_json(json!({"content_type": "form"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let report = apply_integrations(&client, "my-repo", &plan).await.unwrap();
    assert!(
        report
            .applied
            .iter()
            .any(|line| line.contains("Updated webhook"))
    );
}

#[tokio::test]
async fn deploy_key_replace_creates_before_delete() {
    unsafe {
        std::env::set_var("WARD_DEPLOY_KEY", "ssh-rsa AAA");
    }
    let current = IntegrationsCollection {
        category: RepositoryIntegrationsCategoryV2::default(),
        state: CollectedIntegrationsState {
            webhooks: Vec::new(),
            webhooks_complete: true,
            deploy_keys: vec![CollectedDeployKey {
                id: 5,
                config: DeployKeyConfigV2 {
                    title: "readonly".to_owned(),
                    read_only: Some(true),
                    fingerprint: Some("aa:bb".to_owned()),
                    replacement_key: None,
                },
            }],
            deploy_keys_complete: true,
            pages: None,
            pages_complete: true,
            autolinks: Vec::new(),
            autolinks_complete: true,
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let desired = RepositoryIntegrationsCategoryV2 {
        policy: managed_sensitive_policy(false),
        deploy_keys: vec![DeployKeyConfigV2 {
            title: "readonly".to_owned(),
            read_only: Some(false),
            fingerprint: Some("aa:bb".to_owned()),
            replacement_key: Some(ExternalValueReference::Env {
                key: "WARD_DEPLOY_KEY".to_owned(),
            }),
        }],
        ..RepositoryIntegrationsCategoryV2::default()
    };
    let plan = plan_integrations(&current, &desired);
    assert!(matches!(
        plan.deploy_key_actions[0],
        DeployKeyAction::Replace { .. }
    ));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/keys"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 6})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/repos/test-org/my-repo/keys/5"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    apply_integrations(&client, "my-repo", &plan).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let paths = requests
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "/repos/test-org/my-repo/keys",
            "/repos/test-org/my-repo/keys/5"
        ]
    );
    unsafe {
        std::env::remove_var("WARD_DEPLOY_KEY");
    }
}

#[test]
fn pages_status_autolink_recreate_and_idempotence_work() {
    let current = IntegrationsCollection {
        category: RepositoryIntegrationsCategoryV2::default(),
        state: CollectedIntegrationsState {
            webhooks: Vec::new(),
            webhooks_complete: true,
            deploy_keys: Vec::new(),
            deploy_keys_complete: true,
            pages: Some(CollectedPages {
                config: PagesConfigV2 {
                    build_type: Some("workflow".to_owned()),
                    source_branch: None,
                    source_path: None,
                    cname: Some("docs.example.test".to_owned()),
                    https_enforced: Some(true),
                },
                status: Some("building".to_owned()),
            }),
            pages_complete: true,
            autolinks: vec![CollectedAutolink {
                id: 10,
                config: AutolinkConfigV2 {
                    key_prefix: "ABC-".to_owned(),
                    url_template: "https://tracker.example/ABC-<num>".to_owned(),
                    is_alphanumeric: Some(true),
                },
            }],
            autolinks_complete: true,
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let desired = RepositoryIntegrationsCategoryV2 {
        policy: managed_sensitive_policy(false),
        pages: current
            .state
            .pages
            .as_ref()
            .map(|pages| pages.config.clone()),
        autolinks: vec![AutolinkConfigV2 {
            key_prefix: "ABC-".to_owned(),
            url_template: "https://tracker.example/ABC-<num>".to_owned(),
            is_alphanumeric: Some(false),
        }],
        ..RepositoryIntegrationsCategoryV2::default()
    };
    let plan = plan_integrations(&current, &desired);
    assert!(matches!(
        plan.autolink_actions[0],
        ward::reconcile::access_integrations::AutolinkAction::Recreate { .. }
    ));

    let verify = verify_integrations_state(
        &IntegrationsCollection {
            state: CollectedIntegrationsState {
                autolinks: vec![CollectedAutolink {
                    id: 10,
                    config: desired.autolinks[0].clone(),
                }],
                ..current.state.clone()
            },
            ..current.clone()
        },
        &RepositoryIntegrationsCategoryV2 {
            autolinks: vec![desired.autolinks[0].clone()],
            pages: desired.pages.clone(),
            ..desired.clone()
        },
    );
    assert!(verify.issues.is_empty());
    assert!(verify.notes.iter().any(|note| note.contains("building")));
}

#[test]
fn prune_and_sensitive_gates_block_mutations() {
    let current = IntegrationsCollection {
        category: RepositoryIntegrationsCategoryV2::default(),
        state: CollectedIntegrationsState {
            webhooks: vec![CollectedWebhook {
                id: 1,
                canonical_url: canonicalize_url("https://hooks.example.test"),
                config: WebhookConfigV2 {
                    url: "https://hooks.example.test".to_owned(),
                    url_from: None,
                    active: Some(true),
                    events: vec!["push".to_owned()],
                    content_type: Some("json".to_owned()),
                    insecure_ssl: Some(false),
                    secret: None,
                },
            }],
            webhooks_complete: true,
            deploy_keys: Vec::new(),
            deploy_keys_complete: true,
            pages: None,
            pages_complete: true,
            autolinks: Vec::new(),
            autolinks_complete: true,
        },
        coverage: Vec::new(),
        issues: Vec::new(),
    };
    let desired = RepositoryIntegrationsCategoryV2 {
        policy: CategoryPolicy {
            disposition: ManagementDisposition::Managed,
            prune: true,
            sensitive: false,
        },
        ..RepositoryIntegrationsCategoryV2::default()
    };
    let plan = plan_integrations(&current, &desired);
    assert!(plan.is_empty());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.message.contains("require `policy.sensitive: true`"))
    );
}
