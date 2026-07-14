mod common;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    CategoryPolicy, ImmutableReleasesConfig, RepositoryCategoryV2, RepositoryMetadataConfig,
    RepositorySettingsConfig,
};
use ward::github::Client;
use ward::reconcile::general::{
    CollectedGeneralState, GeneralCollectedExtensions, GeneralCustomPropertyValue,
    GeneralDesiredState, GeneralLabel, GeneralPlanOptions, PlannedImmutableReleaseAction,
    PlannedLabelAction, apply, plan, plan_with_options,
};

fn current_state() -> CollectedGeneralState {
    CollectedGeneralState {
        repository: RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: Some(RepositorySettingsConfig {
                has_issues: Some(true),
                has_projects: Some(false),
                has_wiki: Some(true),
                has_discussions: Some(false),
                has_pull_requests: Some(true),
                pull_request_creation_policy: Some("all".to_owned()),
                has_sponsorships_enabled: Some(false),
                issue_creation_policy: Some("all".to_owned()),
                allow_squash_merge: Some(true),
                allow_merge_commit: Some(false),
                allow_rebase_merge: Some(true),
                allow_auto_merge: Some(true),
                delete_branch_on_merge: Some(true),
                allow_update_branch: Some(true),
                squash_merge_commit_title: Some("PR_TITLE".to_owned()),
                squash_merge_commit_message: Some("PR_BODY".to_owned()),
                merge_commit_title: Some("PR_TITLE".to_owned()),
                merge_commit_message: Some("PR_BODY".to_owned()),
                web_commit_signoff_required: Some(true),
                use_squash_pr_title_as_default: Some(false),
                topics: Some(vec!["managed".to_owned()]),
            }),
            metadata: Some(RepositoryMetadataConfig {
                description: Some("Managed repo".to_owned()),
                homepage: Some("https://example.test".to_owned()),
                default_branch: Some("main".to_owned()),
                visibility: Some("public".to_owned()),
                archived: Some(false),
                is_template: Some(false),
                allow_forking: Some(true),
            }),
            custom_properties: Vec::new(),
            immutable_releases: Some(ImmutableReleasesConfig {
                enabled: Some(true),
                enforced_by_owner: Some(false),
            }),
            references: Vec::new(),
        },
        labels: vec![
            GeneralLabel {
                name: "bug".to_owned(),
                color: Some("f29513".to_owned()),
                description: Some("Bug".to_owned()),
                default: true,
            },
            GeneralLabel {
                name: "stale".to_owned(),
                color: Some("ededed".to_owned()),
                description: Some("Old".to_owned()),
                default: false,
            },
        ],
        custom_properties: vec![GeneralCustomPropertyValue {
            property_name: "team".to_owned(),
            value: json!("party"),
        }],
        coverage: Vec::new(),
        extensions: GeneralCollectedExtensions {
            repository_id: "R_kgDOTest".to_owned(),
            graphql_settings_collected: true,
            labels_collected: true,
            custom_properties_collected: true,
            immutable_releases_collected: true,
            has_discussions_enabled: Some(false),
            has_pull_requests: Some(true),
            pull_request_creation_policy: Some("all".to_owned()),
            has_sponsorships_enabled: Some(false),
            issue_creation_policy: Some("all".to_owned()),
            use_squash_pr_title_as_default: Some(false),
        },
    }
}

#[test]
fn high_impact_changes_are_blocked_by_default_and_opt_in_when_requested() {
    let current = current_state();
    let desired = GeneralDesiredState {
        repository: RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: Some(RepositorySettingsConfig {
                has_discussions: Some(true),
                ..RepositorySettingsConfig::default()
            }),
            metadata: Some(RepositoryMetadataConfig {
                visibility: Some("private".to_owned()),
                archived: Some(true),
                ..RepositoryMetadataConfig::default()
            }),
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        },
        labels: Vec::new(),
        custom_properties: Vec::new(),
        extensions: Default::default(),
    };

    let blocked_plan = plan("my-repo", &desired, &current);
    assert!(blocked_plan.rest_patch.as_object().unwrap().is_empty());
    assert_eq!(blocked_plan.blocked_changes.len(), 2);
    assert!(
        blocked_plan
            .blocked_changes
            .iter()
            .all(|change| change.high_impact)
    );

    let allowed_plan = plan_with_options(
        "my-repo",
        &desired,
        &current,
        GeneralPlanOptions {
            allow_high_impact: true,
        },
    );
    assert!(allowed_plan.blocked_changes.is_empty());
    assert!(allowed_plan.options.allow_high_impact);
    assert_eq!(
        allowed_plan.rest_patch["visibility"].as_str(),
        Some("private")
    );
    assert_eq!(allowed_plan.rest_patch["archived"].as_bool(), Some(true));
}

#[tokio::test]
async fn apply_blocks_when_default_branch_does_not_exist() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/branches/release%2F2026.07"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Branch not found"
        })))
        .mount(&server)
        .await;

    let current = current_state();
    let desired = GeneralDesiredState {
        repository: RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: None,
            metadata: Some(RepositoryMetadataConfig {
                default_branch: Some("release/2026.07".to_owned()),
                ..RepositoryMetadataConfig::default()
            }),
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        },
        labels: Vec::new(),
        custom_properties: Vec::new(),
        extensions: Default::default(),
    };
    let planned = plan("my-repo", &desired, &current);

    let client = Client::new_for_test("test-org", &server.uri());
    let error = apply(&client, &planned).await.unwrap_err();

    assert!(error.to_string().contains("branch does not exist"));
}

#[tokio::test]
async fn apply_rejects_mixed_blocked_and_actionable_plan_before_writing() {
    let current = current_state();
    let desired = GeneralDesiredState {
        repository: RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: Some(RepositorySettingsConfig {
                has_issues: Some(false),
                ..RepositorySettingsConfig::default()
            }),
            metadata: Some(RepositoryMetadataConfig {
                visibility: Some("private".to_owned()),
                ..RepositoryMetadataConfig::default()
            }),
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        },
        labels: Vec::new(),
        custom_properties: Vec::new(),
        extensions: Default::default(),
    };
    let planned = plan("my-repo", &desired, &current);
    assert!(planned.has_actionable_changes());
    assert!(planned.has_blocked_changes());

    let server = MockServer::start().await;
    let client = Client::new_for_test("test-org", &server.uri());
    let error = apply(&client, &planned).await.unwrap_err();

    assert!(error.to_string().contains("blocked"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn immutable_releases_enforced_by_owner_are_reference_only() {
    let mut current = current_state();
    current.repository.immutable_releases = Some(ImmutableReleasesConfig {
        enabled: Some(true),
        enforced_by_owner: Some(true),
    });

    let desired = GeneralDesiredState {
        repository: RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: None,
            metadata: None,
            custom_properties: Vec::new(),
            immutable_releases: Some(ImmutableReleasesConfig {
                enabled: Some(false),
                enforced_by_owner: Some(false),
            }),
            references: Vec::new(),
        },
        labels: Vec::new(),
        custom_properties: Vec::new(),
        extensions: Default::default(),
    };

    let planned = plan("my-repo", &desired, &current);

    assert_eq!(
        planned.immutable_releases,
        Some(PlannedImmutableReleaseAction::Reference)
    );
    assert!(!planned.has_actionable_changes());
    assert!(
        planned
            .blocked_changes
            .iter()
            .all(|change| change.reference_only)
    );
}

#[test]
fn label_actions_respect_prune_gate_and_default_label_safety() {
    let current = current_state();
    let desired = GeneralDesiredState {
        repository: RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: None,
            metadata: None,
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        },
        labels: vec![
            GeneralLabel {
                name: "bug".to_owned(),
                color: Some("d73a4a".to_owned()),
                description: Some("Something is broken".to_owned()),
                default: true,
            },
            GeneralLabel {
                name: "feature".to_owned(),
                color: Some("0e8a16".to_owned()),
                description: Some("New feature".to_owned()),
                default: false,
            },
        ],
        custom_properties: Vec::new(),
        extensions: Default::default(),
    };

    let planned_without_prune = plan("my-repo", &desired, &current);
    assert!(
        planned_without_prune
            .label_actions
            .iter()
            .any(|action| matches!(action, PlannedLabelAction::Create { .. }))
    );
    assert!(
        planned_without_prune
            .label_actions
            .iter()
            .any(|action| matches!(action, PlannedLabelAction::Update { .. }))
    );
    assert!(
        !planned_without_prune
            .label_actions
            .iter()
            .any(|action| matches!(action, PlannedLabelAction::Delete { .. }))
    );

    let mut desired_with_prune = desired.clone();
    desired_with_prune.repository.policy.prune = true;
    desired_with_prune.labels = vec![GeneralLabel {
        name: "feature".to_owned(),
        color: Some("0e8a16".to_owned()),
        description: Some("New feature".to_owned()),
        default: false,
    }];
    let planned_with_prune = plan("my-repo", &desired_with_prune, &current);

    assert!(
        planned_with_prune
            .label_actions
            .iter()
            .any(|action| matches!(action, PlannedLabelAction::Delete { name } if name == "stale"))
    );
    assert!(planned_with_prune.blocked_changes.iter().any(|change| {
        matches!(
            &change.kind,
            ward::reconcile::general::GeneralChangeKind::Label { name, action }
                if name == "bug" && *action == ward::reconcile::general::GeneralResourceAction::Delete
        )
    }));
}

#[test]
fn plans_multi_select_custom_properties_and_null_clears() {
    let current = current_state();
    let desired = GeneralDesiredState {
        repository: RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: Some(RepositorySettingsConfig {
                ..RepositorySettingsConfig::default()
            }),
            metadata: Some(RepositoryMetadataConfig {
                description: Some(String::new()),
                homepage: Some(String::new()),
                ..RepositoryMetadataConfig::default()
            }),
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        },
        labels: Vec::new(),
        custom_properties: vec![GeneralCustomPropertyValue {
            property_name: "systems".to_owned(),
            value: json!(["ward", "party"]),
        }],
        extensions: Default::default(),
    };

    let planned = plan("my-repo", &desired, &current);

    assert_eq!(planned.rest_patch["description"].as_str(), Some(""));
    assert_eq!(planned.rest_patch["homepage"].as_str(), Some(""));
    assert_eq!(planned.custom_property_updates.len(), 1);
    assert!(planned.custom_property_updates.iter().any(|update| {
        update.property_name == "systems"
            && update.value.as_ref() == Some(&json!(["ward", "party"]))
    }));
}

#[test]
fn plan_is_idempotent_when_state_matches_desired() {
    let current = current_state();
    let desired = GeneralDesiredState {
        repository: current.repository.clone(),
        labels: current.labels.clone(),
        custom_properties: current.custom_properties.clone(),
        extensions: ward::reconcile::general::GeneralDesiredExtensions {
            has_pull_requests: Some(true),
            pull_request_creation_policy: Some("all".to_owned()),
            has_sponsorships_enabled: Some(false),
            issue_creation_policy: Some("all".to_owned()),
            use_squash_pr_title_as_default: Some(false),
        },
    };

    let planned = plan("my-repo", &desired, &current);

    assert!(!planned.has_actionable_changes());
    assert!(!planned.has_blocked_changes());
    assert!(planned.changes.is_empty());
}
