use super::apply::*;
use super::gating::*;
use super::model::*;
use super::plan::*;
use super::report::*;
use super::*;
use crate::config::Manifest;
use crate::config::manifest::{
    CategoryPolicy, CoverageEntry, CoverageOutcome, ManagementDisposition, RepositoryCategory,
};
use crate::reconcile::{access_integrations, actions_environments, files, security_rules};

#[test]
fn empty_selection_is_all_categories() {
    let all = select_categories(&[]);
    assert_eq!(all.len(), 9);
    assert!(all.contains(&Category::Files));
}

#[test]
fn category_selection_deduplicates() {
    let selected = select_categories(&[Category::Security, Category::Security]);
    assert_eq!(selected, vec![Category::Security]);
}

#[test]
fn coverage_counts_split_by_outcome() {
    use crate::config::manifest::ManifestCategoryName;
    let coverage = vec![
        CoverageEntry {
            category: ManifestCategoryName::Security,
            endpoint: "a".to_owned(),
            outcome: CoverageOutcome::Collected,
            reason: None,
            required_permission: None,
        },
        CoverageEntry {
            category: ManifestCategoryName::Security,
            endpoint: "b".to_owned(),
            outcome: CoverageOutcome::PermissionDenied,
            reason: None,
            required_permission: None,
        },
    ];
    let counts = coverage_counts(&coverage);
    assert_eq!(counts.total, 2);
    assert_eq!(counts.collected, 1);
    assert_eq!(counts.degraded, 1);
    assert_eq!(degraded_coverage(&coverage), 1);
}

#[test]
fn known_github_limits_are_not_counted_as_degraded() {
    use crate::config::manifest::ManifestCategoryName;
    let entry = |outcome| CoverageEntry {
        category: ManifestCategoryName::Repository,
        endpoint: "x".to_owned(),
        outcome,
        reason: None,
        required_permission: None,
    };
    let coverage = vec![
        entry(CoverageOutcome::Collected),
        entry(CoverageOutcome::Unsupported),
        entry(CoverageOutcome::Redacted),
        entry(CoverageOutcome::NotApplicable),
        entry(CoverageOutcome::Unavailable),
    ];

    let counts = coverage_counts(&coverage);

    assert_eq!(counts.degraded, 1);
    assert_eq!(counts.unsupported, 2);
    assert_eq!(degraded_coverage(&coverage), 1);
}

fn empty_files_plan() -> files::FilesPlan {
    files::FilesPlan {
        upserts: Vec::new(),
        deletions: Vec::new(),
        unchanged: Vec::new(),
        atomic_entries: Vec::new(),
        issues: Vec::new(),
    }
}

fn planned_category(
    name: Category,
    disposition: ManagementDisposition,
    actionable: usize,
    blocked: usize,
) -> CategoryPlan {
    CategoryPlan {
        name,
        disposition,
        is_blocked_collection: false,
        coverage: Vec::new(),
        actionable,
        blocked,
        warnings: 0,
        details: Vec::new(),
        kind: CategoryPlanKind::Files(empty_files_plan()),
    }
}

#[test]
fn plan_status_reflects_disposition_and_counts() {
    assert_eq!(
        planned_category(Category::Files, ManagementDisposition::Managed, 3, 0).plan_status(),
        "planned"
    );
    assert_eq!(
        planned_category(Category::Files, ManagementDisposition::Managed, 0, 0).plan_status(),
        "noop"
    );
    assert_eq!(
        planned_category(Category::Files, ManagementDisposition::Observe, 0, 0).plan_status(),
        "observed"
    );
    assert_eq!(
        planned_category(Category::Files, ManagementDisposition::Managed, 2, 1).plan_status(),
        "blocked"
    );
    assert_eq!(absent_category(Category::Files).plan_status(), "skipped");
    assert_eq!(
        collection_failed(
            Category::Files,
            ManagementDisposition::Managed,
            &anyhow::anyhow!("boom"),
        )
        .plan_status(),
        "blocked"
    );
}

#[test]
fn observe_category_reports_zero_actions_and_no_error() {
    let category = planned_category(Category::Access, ManagementDisposition::Observe, 0, 0);
    let report = category.to_report(category.plan_status(), 0, None);
    assert_eq!(report.status, "observed");
    assert_eq!(report.actionable, 0);
    assert_eq!(report.disposition, "observe");
    assert!(report.error.is_none());
}

#[test]
fn collection_failure_is_surfaced_not_hidden() {
    let category = collection_failed(
        Category::Security,
        ManagementDisposition::Managed,
        &anyhow::anyhow!("HTTP 500"),
    );
    let report = category.to_report("blocked", 0, None);
    assert_eq!(report.status, "blocked");
    assert_eq!(report.blocked, 1);
    assert_eq!(report.disposition, "blocked");
    assert!(report.error.as_deref().unwrap().contains("HTTP 500"));
}

#[test]
fn partial_collector_failure_keeps_other_categories() {
    // One category fails collection, an unrelated one still plans.
    let plan = RepoPlan {
        repo: "repo-a".to_owned(),
        default_branch: "main".to_owned(),
        categories: vec![
            collection_failed(
                Category::Security,
                ManagementDisposition::Managed,
                &anyhow::anyhow!("collect failed"),
            ),
            planned_category(Category::Files, ManagementDisposition::Managed, 1, 0),
        ],
    };
    let report = plan.to_report();
    assert_eq!(report.categories.len(), 2);
    let security = &report.categories[0];
    let files = &report.categories[1];
    assert_eq!(security.status, "blocked");
    assert!(security.error.is_some());
    assert_eq!(files.status, "planned");
    assert_eq!(files.actionable, 1);
    assert_eq!(report.blocked, 1);
    assert_eq!(report.actionable, 1);
}

#[test]
fn unified_report_aggregates_and_detects_failures() {
    let ok = aggregate_repo(
        "repo-ok".to_owned(),
        vec![
            planned_category(Category::Files, ManagementDisposition::Managed, 2, 0).to_report(
                "success",
                0,
                Some(true),
            ),
        ],
    );
    let bad = aggregate_repo(
        "repo-bad".to_owned(),
        vec![
            planned_category(Category::Security, ManagementDisposition::Managed, 1, 1)
                .to_report("blocked", 0, None),
        ],
    );
    let report = UnifiedReport::from_repos(vec![ok, bad]);
    assert_eq!(report.actionable, 3);
    assert_eq!(report.blocked, 1);
    assert!(report.has_failures());

    let clean = UnifiedReport::from_repos(vec![aggregate_repo(
        "repo".to_owned(),
        vec![
            planned_category(Category::Files, ManagementDisposition::Managed, 0, 0)
                .to_report("noop", 0, None),
        ],
    )]);
    assert!(!clean.has_failures());
}

#[test]
fn json_summary_shape_is_stable() {
    let report = UnifiedReport::from_repos(vec![aggregate_repo(
        "repo-a".to_owned(),
        vec![
            planned_category(Category::Repository, ManagementDisposition::Managed, 1, 0)
                .to_report("planned", 0, None),
        ],
    )]);
    let value = serde_json::to_value(&report).unwrap();
    assert!(value.get("repos").is_some());
    assert_eq!(value["actionable"], 1);
    assert_eq!(value["blocked"], 0);
    assert_eq!(value["deferred"], 0);
    assert!(value.get("warnings").is_some());
    assert!(value.get("coverage").is_some());
    let category = &value["repos"][0]["categories"][0];
    assert_eq!(category["category"], "repository");
    assert_eq!(category["disposition"], "managed");
    assert_eq!(category["status"], "planned");
    assert!(category.get("coverage").is_some());
    assert!(category.get("coverage_outcomes").is_some());
    assert!(category.get("details").is_some());
}

#[test]
fn actions_safe_subset_defers_workflow_toggles() {
    let plan = actions_environments::ActionsPlan {
        settings_changes: Vec::new(),
        workflow_state_changes: vec![actions_environments::WorkflowStateChange {
            path: ".github/workflows/ci.yml".to_owned(),
            enabled: true,
        }],
        variable_upserts: Vec::new(),
        variable_deletions: Vec::new(),
        secret_upserts: Vec::new(),
        secret_deletions: Vec::new(),
        reference_actions: Vec::new(),
        issues: vec![actions_environments::ReconcileIssue {
            scope: "actions.workflows..github/workflows/missing.yml".to_owned(),
            severity: actions_environments::IssueSeverity::Blocker,
            message: "missing".to_owned(),
        }],
    };
    let (safe, deferred) = actions_safe_subset(&plan);
    assert_eq!(deferred, 2);
    assert!(safe.workflow_state_changes.is_empty());
    assert!(safe.issues.is_empty());

    let (direct, deferred) = actions_plan_for_apply(&plan, false);
    assert_eq!(deferred, 0);
    assert_eq!(direct.workflow_state_changes.len(), 1);
    assert_eq!(direct.issues.len(), 1);
}

#[test]
fn planned_file_pr_reclassifies_missing_workflow_as_deferred() {
    let actions = CategoryPlan {
        name: Category::Actions,
        disposition: ManagementDisposition::Managed,
        is_blocked_collection: false,
        coverage: Vec::new(),
        actionable: 0,
        blocked: 1,
        warnings: 0,
        details: Vec::new(),
        kind: CategoryPlanKind::Actions(actions_environments::ActionsPlan {
            settings_changes: Vec::new(),
            workflow_state_changes: Vec::new(),
            variable_upserts: Vec::new(),
            variable_deletions: Vec::new(),
            secret_upserts: Vec::new(),
            secret_deletions: Vec::new(),
            reference_actions: Vec::new(),
            issues: vec![actions_environments::ReconcileIssue {
                scope: "actions.workflows..github/workflows/ci.yml".to_owned(),
                severity: actions_environments::IssueSeverity::Blocker,
                message: "Workflow file not found".to_owned(),
            }],
        }),
    };
    let plan = RepoPlan {
        repo: "repo".to_owned(),
        default_branch: "main".to_owned(),
        categories: vec![
            planned_category(Category::Files, ManagementDisposition::Managed, 1, 0),
            actions,
        ],
    };

    let report = plan.to_report();
    let actions = &report.categories[1];
    assert_eq!(actions.status, "deferred");
    assert_eq!(actions.blocked, 0);
    assert_eq!(actions.deferred, 1);
    assert_eq!(report.blocked, 0);
}

#[test]
fn integrations_safe_subset_defers_pages() {
    let plan = access_integrations::IntegrationsPlan {
        policy: CategoryPolicy::managed(),
        webhook_actions: Vec::new(),
        deploy_key_actions: Vec::new(),
        pages_action: Some(access_integrations::PagesAction::Delete),
        autolink_actions: Vec::new(),
        notes: Vec::new(),
        issues: Vec::new(),
    };
    let (safe, deferred) = integrations_safe_subset(&plan);
    assert_eq!(deferred, 1);
    assert!(safe.pages_action.is_none());

    let (direct, deferred) = integrations_plan_for_apply(&plan, false);
    assert_eq!(deferred, 0);
    assert!(direct.pages_action.is_some());
}

#[test]
fn files_verification_mismatch_is_a_failed_report() {
    let category = planned_category(Category::Files, ManagementDisposition::Managed, 1, 0);
    let report = files_apply_report(
        &category,
        vec!["PR branch does not match desired state after commit".to_owned()],
        false,
    );

    assert_eq!(report.status, "failed");
    assert_eq!(report.verified, Some(false));
    assert!(report.configuration_pull_request_pending);
    assert!(report.details[0].contains("does not match"));
}

#[test]
fn unreadable_config_pr_lookup_degrades_dependent_categories_to_coverage() {
    let mut plan = RepoPlan {
        repo: "repo".to_owned(),
        default_branch: "main".to_owned(),
        categories: vec![CategoryPlan {
            name: Category::Rulesets,
            disposition: ManagementDisposition::Managed,
            is_blocked_collection: false,
            coverage: Vec::new(),
            actionable: 3,
            blocked: 0,
            warnings: 0,
            details: Vec::new(),
            kind: CategoryPlanKind::Rulesets(security_rules::RulesetsPlan {
                actions: Vec::new(),
                issues: Vec::new(),
            }),
        }],
    };

    plan.record_config_pr_lookup_failure("HTTP 403");

    let report = UnifiedReport::from_repos(vec![plan.to_report()]);
    assert!(report.has_unknown_managed_state());
    assert_eq!(report.warnings, 1);
}

#[test]
fn rulesets_and_branch_protection_defer_behind_config_pr() {
    let rulesets = CategoryPlan {
        name: Category::Rulesets,
        disposition: ManagementDisposition::Managed,
        is_blocked_collection: false,
        coverage: Vec::new(),
        actionable: 3,
        blocked: 0,
        warnings: 0,
        details: Vec::new(),
        kind: CategoryPlanKind::Rulesets(security_rules::RulesetsPlan {
            actions: Vec::new(),
            issues: Vec::new(),
        }),
    };
    let branch_protection = CategoryPlan {
        name: Category::BranchProtection,
        disposition: ManagementDisposition::Managed,
        is_blocked_collection: false,
        coverage: Vec::new(),
        actionable: 2,
        blocked: 0,
        warnings: 0,
        details: Vec::new(),
        kind: CategoryPlanKind::BranchProtection(security_rules::BranchProtectionPlan {
            actions: Vec::new(),
            issues: Vec::new(),
        }),
    };

    assert_eq!(dependency_deferral(&rulesets, true).total, 3);
    assert_eq!(dependency_deferral(&branch_protection, true).total, 2);
    assert_eq!(dependency_deferral(&rulesets, false).total, 0);
}

#[test]
fn build_general_desired_merges_integration_labels_under_repository_policy() {
    use crate::config::manifest::{
        LabelConfig, RepositoryCategory, RepositoryIntegrationsCategory,
    };

    let mut manifest = Manifest::default();
    manifest.categories.repository = Some(RepositoryCategory {
        policy: CategoryPolicy::managed(),
        settings: None,
        metadata: None,
        custom_properties: Vec::new(),
        immutable_releases: None,
        references: Vec::new(),
    });
    manifest.categories.integrations = Some(RepositoryIntegrationsCategory {
        policy: CategoryPolicy::observe_sensitive(),
        labels: vec![LabelConfig {
            name: "bug".to_owned(),
            color: Some("d73a4a".to_owned()),
            description: Some("Something is broken".to_owned()),
            default: Some(true),
        }],
        ..RepositoryIntegrationsCategory::default()
    });

    let desired = build_general_desired(&manifest.categories).unwrap();
    assert_eq!(desired.labels.len(), 1);
    assert_eq!(desired.labels[0].name, "bug");
    // Repository policy governs labels: managed here.
    assert_eq!(
        desired.repository.policy.disposition,
        ManagementDisposition::Managed
    );
}

#[test]
fn apply_order_covers_all_categories_repository_first_protection_last() {
    let order = Category::apply_order();
    assert_eq!(order.len(), 9);
    assert_eq!(order[0], Category::Repository);
    assert_eq!(order[1], Category::Files);
    assert_eq!(order[8], Category::BranchProtection);
}

#[test]
fn unrequested_optional_repository_reads_do_not_count_as_unknown_state() {
    let entry = |endpoint: &str| CoverageEntry {
        category: crate::config::manifest::ManifestCategoryName::Repository,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::PermissionDenied,
        reason: Some("HTTP 403".to_owned()),
        required_permission: None,
    };
    let mut coverage = vec![
        entry("GET /repos/{owner}/{repo}/properties/values"),
        entry("GET /repos/{owner}/{repo}/immutable-releases"),
        entry("GET /repos/{owner}/{repo}/topics"),
    ];
    let desired = RepositoryCategory {
        policy: CategoryPolicy::managed(),
        settings: None,
        metadata: None,
        custom_properties: Vec::new(),
        immutable_releases: None,
        references: Vec::new(),
    };

    relax_unrequested_repository_coverage(&mut coverage, &desired);

    assert_eq!(coverage[0].outcome, CoverageOutcome::NotApplicable);
    assert_eq!(coverage[1].outcome, CoverageOutcome::NotApplicable);
    assert_eq!(coverage[2].outcome, CoverageOutcome::PermissionDenied);
}
