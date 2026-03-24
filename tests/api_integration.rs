use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::cli::drift::{compare_protection, compare_security};
use ward::config::manifest::{BranchProtectionConfig, SecurityConfig};
use ward::github::Client;
use ward::github::branch_protection::BranchProtectionState;
use ward::github::commits::CommitFile;
use ward::github::security::SecurityState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_repo_json(name: &str, archived: bool) -> serde_json::Value {
    json!({
        "name": name,
        "full_name": format!("test-org/{name}"),
        "archived": archived,
        "default_branch": "main",
        "description": format!("Repo {name}"),
        "visibility": "private",
        "language": "Kotlin"
    })
}

// ===========================================================================
// A. Repository listing
// ===========================================================================

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
                make_repo_json("s07252-foo", false),
                make_repo_json("s07252-bar", false),
                make_repo_json("s07313-baz", false),
                make_repo_json("s07252-archived", true),
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
        .list_repos_for_system("s07252", &[], &[])
        .await
        .unwrap();

    assert_eq!(repos.len(), 2);
    assert!(repos.iter().all(|r| r.name.starts_with("s07252")));
    assert!(repos.iter().all(|r| !r.archived));
}

// ===========================================================================
// B. Security state
// ===========================================================================

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

// ===========================================================================
// C. Branch protection
// ===========================================================================

#[tokio::test]
async fn test_get_branch_protection_exists() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "required_pull_request_reviews": {
                "required_approving_review_count": 2,
                "dismiss_stale_reviews": true,
                "require_code_owner_reviews": true
            },
            "required_status_checks": {
                "strict": true,
                "contexts": []
            },
            "enforce_admins": { "enabled": true },
            "required_linear_history": { "enabled": false },
            "allow_force_pushes": { "enabled": false },
            "allow_deletions": { "enabled": false }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let protection = client
        .get_branch_protection("my-repo", "main")
        .await
        .unwrap();

    let state = protection.expect("should return Some");
    assert!(state.required_pull_request_reviews);
    assert_eq!(state.required_approving_review_count, 2);
    assert!(state.dismiss_stale_reviews);
    assert!(state.require_code_owner_reviews);
    assert!(state.required_status_checks);
    assert!(state.strict_status_checks);
    assert!(state.enforce_admins);
    assert!(!state.required_linear_history);
    assert!(!state.allow_force_pushes);
    assert!(!state.allow_deletions);
}

#[tokio::test]
async fn test_get_branch_protection_none() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Branch not protected"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let protection = client
        .get_branch_protection("my-repo", "main")
        .await
        .unwrap();

    assert!(protection.is_none());
}

#[tokio::test]
async fn test_update_branch_protection() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/repos/test-org/my-repo/branches/main/protection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": "https://api.github.com/repos/test-org/my-repo/branches/main/protection"
        })))
        .mount(&server)
        .await;

    let config = BranchProtectionConfig {
        enabled: true,
        required_approvals: 1,
        dismiss_stale_reviews: true,
        require_code_owner_reviews: false,
        require_status_checks: true,
        strict_status_checks: true,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .update_branch_protection("my-repo", "main", &config)
        .await
        .unwrap();
}

// ===========================================================================
// D. Rulesets
// ===========================================================================

#[tokio::test]
async fn test_list_rulesets() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/rulesets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 1, "name": "Branch protection" },
            { "id": 2, "name": "Copilot Code Review" }
        ])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let rulesets = client.list_rulesets("my-repo").await.unwrap();

    assert_eq!(rulesets.len(), 2);
    assert_eq!(rulesets[0].id, 1);
    assert_eq!(rulesets[0].name, "Branch protection");
    assert_eq!(rulesets[1].id, 2);
    assert_eq!(rulesets[1].name, "Copilot Code Review");
}

#[tokio::test]
async fn test_get_ruleset_detail() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/rulesets/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 123,
            "name": "Branch protection",
            "enforcement": "active",
            "target": "branch",
            "rules": [
                {
                    "type": "pull_request",
                    "parameters": {
                        "required_approving_review_count": 1,
                        "dismiss_stale_reviews_on_push": false
                    }
                }
            ],
            "conditions": {
                "ref_name": {
                    "include": ["~DEFAULT_BRANCH"],
                    "exclude": []
                }
            },
            "bypass_actors": []
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let detail = client.get_ruleset("my-repo", 123).await.unwrap();

    assert_eq!(detail.id, 123);
    assert_eq!(detail.name, "Branch protection");
    assert_eq!(detail.enforcement, "active");
    assert_eq!(detail.target, "branch");
    assert_eq!(detail.rules.len(), 1);
    assert_eq!(detail.rules[0].rule_type, "pull_request");
}

#[tokio::test]
async fn test_create_ruleset() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/rulesets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 456,
            "name": "Copilot Code Review",
            "enforcement": "active",
            "target": "branch",
            "rules": [{
                "type": "copilot_code_review",
                "parameters": { "review_on_push": true }
            }],
            "conditions": null,
            "bypass_actors": []
        })))
        .mount(&server)
        .await;

    let body = json!({
        "name": "Copilot Code Review",
        "target": "branch",
        "enforcement": "active",
        "rules": [{
            "type": "copilot_code_review",
            "parameters": { "review_on_push": true }
        }]
    });

    let client = Client::new_for_test("test-org", &server.uri());
    let created = client.create_ruleset("my-repo", &body).await.unwrap();

    assert_eq!(created.id, 456);
    assert_eq!(created.name, "Copilot Code Review");
}

// ===========================================================================
// E. Teams
// ===========================================================================

#[tokio::test]
async fn test_list_org_teams() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/orgs/test-org/teams"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 1,
                "name": "DevOps",
                "slug": "devops",
                "description": "Platform team",
                "permission": "push",
                "privacy": "closed"
            },
            {
                "id": 2,
                "name": "Backend",
                "slug": "backend",
                "description": "Backend team",
                "permission": "push",
                "privacy": "closed"
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/orgs/test-org/teams"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let teams = client.list_org_teams().await.unwrap();

    assert_eq!(teams.len(), 2);
    assert_eq!(teams[0].slug, "devops");
    assert_eq!(teams[1].slug, "backend");
}

#[tokio::test]
async fn test_list_repo_teams() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "id": 1,
                "name": "DevOps",
                "slug": "devops",
                "description": "Platform team",
                "permission": "admin",
                "privacy": "closed"
            }
        ])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let teams = client.list_repo_teams("my-repo").await.unwrap();

    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].slug, "devops");
    assert_eq!(teams[0].permission, "admin");
}

#[tokio::test]
async fn test_add_team_to_repo() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/orgs/test-org/teams/devops/repos/test-org/my-repo"))
        .and(body_partial_json(json!({ "permission": "push" })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .add_team_to_repo("my-repo", "devops", "push")
        .await
        .unwrap();
}

// ===========================================================================
// F. Drift detection logic
// ===========================================================================

#[tokio::test]
async fn test_drift_check_detects_security_drift() {
    let desired = SecurityConfig {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: true,
        dependabot_alerts: true,
        dependabot_security_updates: true,
        codeql_advanced_setup: false,
    };
    let actual = SecurityState {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: false, // drifted
        dependabot_alerts: true,
        dependabot_security_updates: true,
    };

    let drifts = compare_security(&desired, &actual);

    assert_eq!(drifts.len(), 1);
    assert_eq!(drifts[0].field, "push_protection");
    assert_eq!(drifts[0].expected, "true");
    assert_eq!(drifts[0].actual, "false");
}

#[tokio::test]
async fn test_drift_check_no_drift() {
    let desired_sec = SecurityConfig {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: true,
        dependabot_alerts: true,
        dependabot_security_updates: true,
        codeql_advanced_setup: false,
    };
    let actual_sec = SecurityState {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: true,
        dependabot_alerts: true,
        dependabot_security_updates: true,
    };

    let desired_prot = BranchProtectionConfig {
        enabled: true,
        required_approvals: 1,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        require_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    let actual_prot = BranchProtectionState {
        required_pull_request_reviews: true,
        required_approving_review_count: 1,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        required_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };

    assert!(compare_security(&desired_sec, &actual_sec).is_empty());
    assert!(compare_protection(&desired_prot, &actual_prot).is_empty());
}

#[tokio::test]
async fn test_drift_check_detects_protection_drift() {
    let desired = BranchProtectionConfig {
        enabled: true,
        required_approvals: 2,
        dismiss_stale_reviews: true,
        require_code_owner_reviews: false,
        require_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    let actual = BranchProtectionState {
        required_pull_request_reviews: true,
        required_approving_review_count: 1, // wrong
        dismiss_stale_reviews: false,       // wrong
        require_code_owner_reviews: false,
        required_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };

    let drifts = compare_protection(&desired, &actual);

    assert_eq!(drifts.len(), 2);
    let fields: Vec<&str> = drifts.iter().map(|d| d.field.as_str()).collect();
    assert!(fields.contains(&"dismiss_stale_reviews"));
    assert!(fields.contains(&"required_approvals"));
}

// ===========================================================================
// G. Commits API
// ===========================================================================

#[tokio::test]
async fn test_create_commit() {
    let server = MockServer::start().await;

    // 1. GET ref
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "abc123", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // 2. GET commit (to get tree SHA)
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/commits/abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "abc123",
            "tree": { "sha": "tree-sha-000" },
            "message": "initial commit"
        })))
        .mount(&server)
        .await;

    // 3. POST blob
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/blobs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "blob-sha-001"
        })))
        .mount(&server)
        .await;

    // 4. POST tree
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/trees"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "new-tree-sha"
        })))
        .mount(&server)
        .await;

    // 5. POST commit
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/commits"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "new-commit-sha"
        })))
        .mount(&server)
        .await;

    // 6. PATCH ref
    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/my-repo/git/refs/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "new-commit-sha" }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let files = vec![CommitFile {
        path: "README.md".to_owned(),
        content: "# Hello\nWorld".to_owned(),
    }];

    let sha = client
        .create_commit("my-repo", "main", "test commit", &files)
        .await
        .unwrap();

    assert_eq!(sha, "new-commit-sha");
}

#[tokio::test]
async fn test_create_branch() {
    let server = MockServer::start().await;

    // GET ref for source branch
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "source-sha-123", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // POST refs for new branch
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/refs"))
        .and(body_partial_json(json!({
            "ref": "refs/heads/feature",
            "sha": "source-sha-123"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "ref": "refs/heads/feature",
            "object": { "sha": "source-sha-123" }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .create_branch("my-repo", "feature", "main")
        .await
        .unwrap();
}

// ===========================================================================
// H. Pull requests
// ===========================================================================

#[tokio::test]
async fn test_create_pull_request_new() {
    let server = MockServer::start().await;

    // No existing PR
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/pulls"))
        .and(query_param("state", "open"))
        .and(query_param("head", "test-org:feature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    // Create PR
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/pulls"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "number": 42,
            "html_url": "https://github.com/test-org/my-repo/pull/42",
            "state": "open",
            "title": "chore: ward setup",
            "head": { "ref": "feature" }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let pr = client
        .create_pull_request(
            "my-repo",
            "chore: ward setup",
            "Automated by ward",
            "feature",
            "main",
            &[],
        )
        .await
        .unwrap();

    assert_eq!(pr.number, 42);
    assert_eq!(pr.state, "open");
    assert_eq!(pr.head.branch, "feature");
}

#[tokio::test]
async fn test_create_pull_request_already_exists() {
    let server = MockServer::start().await;

    // Existing PR found
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/pulls"))
        .and(query_param("state", "open"))
        .and(query_param("head", "test-org:feature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "number": 99,
                "html_url": "https://github.com/test-org/my-repo/pull/99",
                "state": "open",
                "title": "chore: existing PR",
                "head": { "ref": "feature" }
            }
        ])))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let pr = client
        .create_pull_request(
            "my-repo",
            "chore: ward setup",
            "Automated by ward",
            "feature",
            "main",
            &[],
        )
        .await
        .unwrap();

    // Should return the existing PR, not create a new one
    assert_eq!(pr.number, 99);
    assert_eq!(pr.title, "chore: existing PR");
}

// ===========================================================================
// I. File contents
// ===========================================================================

#[tokio::test]
async fn test_get_file_exists() {
    let server = MockServer::start().await;

    let content_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        "# Instructions\nUse ward.",
    );

    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/contents/.github/copilot-instructions.md",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "copilot-instructions.md",
            "path": ".github/copilot-instructions.md",
            "sha": "file-sha-abc",
            "content": content_b64,
            "encoding": "base64"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let file = client
        .get_file("my-repo", ".github/copilot-instructions.md", None)
        .await
        .unwrap();

    let content = file.expect("should return Some");
    assert_eq!(content.name, "copilot-instructions.md");
    assert_eq!(content.path, ".github/copilot-instructions.md");
    assert_eq!(content.sha, "file-sha-abc");

    let decoded = Client::decode_content(&content).unwrap();
    assert_eq!(decoded, "# Instructions\nUse ward.");
}

#[tokio::test]
async fn test_get_file_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/contents/.github/copilot-instructions.md",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let result = client
        .get_file("my-repo", ".github/copilot-instructions.md", None)
        .await
        .unwrap();

    assert!(result.is_none());
}
