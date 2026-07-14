use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    ActorReference, BranchProtectionCategoryV2, CategoryPolicy, ManagementDisposition,
};
use ward::github::Client;
use ward::reconcile::security_rules::{
    BranchProtectionPlanAction, collect_branch_protection_category,
    plan_branch_protection_category, verify_branch_protection_category,
};

#[tokio::test]
async fn security_rules_branch_protection_enumerates_all_branches_and_round_trips() {
    let server = MockServer::start().await;
    let repo = json!({
        "name": "example",
        "full_name": "test-org/example",
        "archived": false,
        "default_branch": "main",
        "visibility": "private"
    });

    Mock::given(method("GET"))
        .and(path("/repos/test-org/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/branches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "name": "main" },
            { "name": "release/1.0" }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "required_status_checks": {
                "strict": true,
                "contexts": ["ci"],
                "checks": [{ "context": "ci", "app_id": 17 }]
            },
            "required_pull_request_reviews": {
                "dismissal_restrictions": {
                    "users": [{ "login": "alice" }],
                    "teams": [{ "slug": "platform" }],
                    "apps": [{ "slug": "release-bot" }]
                },
                "bypass_pull_request_allowances": {
                    "users": [],
                    "teams": [{ "slug": "platform" }],
                    "apps": []
                },
                "dismiss_stale_reviews": true,
                "require_code_owner_reviews": true,
                "required_approving_review_count": 2,
                "require_last_push_approval": true
            },
            "enforce_admins": { "enabled": true },
            "restrictions": {
                "users": [{ "login": "alice" }],
                "teams": [{ "slug": "platform" }],
                "apps": [{ "slug": "release-bot" }]
            },
            "required_linear_history": { "enabled": true },
            "allow_force_pushes": { "enabled": false },
            "allow_deletions": { "enabled": false },
            "block_creations": { "enabled": true },
            "required_conversation_resolution": { "enabled": true },
            "required_signatures": { "enabled": true },
            "lock_branch": { "enabled": false },
            "allow_fork_syncing": { "enabled": false }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/example/branches/release%2F1.0/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "required_status_checks": {
                "strict": false,
                "contexts": ["release-ci"],
                "checks": [{ "context": "release-ci", "app_id": null }]
            },
            "required_pull_request_reviews": {
                "dismissal_restrictions": { "users": [], "teams": [], "apps": [] },
                "bypass_pull_request_allowances": { "users": [], "teams": [], "apps": [] },
                "dismiss_stale_reviews": false,
                "require_code_owner_reviews": true,
                "required_approving_review_count": 1
            },
            "enforce_admins": { "enabled": true },
            "restrictions": { "users": [], "teams": [{ "slug": "release-engineering" }], "apps": [] },
            "required_linear_history": { "enabled": true },
            "allow_force_pushes": { "enabled": false },
            "allow_deletions": { "enabled": false },
            "required_conversation_resolution": { "enabled": true },
            "required_signatures": { "enabled": true },
            "lock_branch": { "enabled": false },
            "allow_fork_syncing": { "enabled": false }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_branch_protection_category(&client, "example", None)
        .await
        .unwrap();

    assert_eq!(collected.default_branch_name, "main");
    assert_eq!(collected.category.protected_branches.len(), 1);
    assert_eq!(collected.category.protected_branches[0].name, "release/1.0");
    let default_branch = collected.category.default_branch_detailed.as_ref().unwrap();
    assert_eq!(default_branch.status_checks.len(), 1);
    assert_eq!(default_branch.status_checks[0].context, "ci");
    assert_eq!(default_branch.status_checks[0].app_id, Some(17));
    assert_eq!(default_branch.require_last_push_approval, Some(true));
    assert_eq!(default_branch.block_creations, Some(true));
    assert_eq!(
        default_branch.pull_request_bypass_allowances,
        vec![ActorReference::Team {
            slug: "platform".to_owned()
        }]
    );

    let round_trip = verify_branch_protection_category(&client, "example", &collected.category)
        .await
        .unwrap();
    assert!(round_trip.matches);

    let desired = BranchProtectionCategoryV2 {
        policy: CategoryPolicy {
            disposition: ManagementDisposition::Managed,
            prune: true,
            sensitive: false,
        },
        default_branch: collected.category.default_branch.clone(),
        default_branch_detailed: None,
        protected_branches: Vec::new(),
    };
    let plan = plan_branch_protection_category(&desired, &collected).unwrap();
    assert!(plan
        .actions
        .iter()
        .any(|action| matches!(action, BranchProtectionPlanAction::Delete { branch } if branch == "release/1.0")));
}
