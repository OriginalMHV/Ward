mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::Manifest;
use ward::config::manifest::{
    CategoryPolicy, FileEncoding, FilesCategoryV2, ManagedFileV2, ManifestSchema,
    RepositoryCategoryV2, RepositoryMetadataConfig, SecurityCategoryV2,
};
use ward::github::Client;
use ward::github::repos::Repository;
use ward::reconcile::unified::{self, Category, UnifiedOptions};

fn base_manifest() -> Manifest {
    let mut manifest = Manifest::default();
    manifest.org.name = "test-org".to_owned();
    manifest.schema = ManifestSchema::current();
    manifest
}

fn managed_files_category(entries: Vec<ManagedFileV2>) -> FilesCategoryV2 {
    FilesCategoryV2 {
        policy: CategoryPolicy::managed(),
        include: Vec::new(),
        exclude: Vec::new(),
        entries,
    }
}

fn dependabot_entry(content: &str) -> ManagedFileV2 {
    ManagedFileV2 {
        path: ".github/dependabot.yml".to_owned(),
        content: content.to_owned(),
        encoding: FileEncoding::Utf8,
        mode: "100644".to_owned(),
        source_sha: None,
    }
}

fn test_repo(name: &str) -> Repository {
    serde_json::from_value(common::make_repo_json(name, false)).unwrap()
}

fn options(categories: Vec<Category>, allow_high_impact: bool) -> UnifiedOptions {
    UnifiedOptions {
        categories,
        allow_high_impact,
        verify: true,
    }
}

/// Mock the default-branch git tree read (GET repo -> ref -> commit -> tree).
async fn mock_default_tree(server: &MockServer, repo: &str, tree_entries: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": repo,
            "full_name": format!("test-org/{repo}"),
            "archived": false,
            "default_branch": "main",
            "visibility": "private"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/git/ref/heads/main")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "commit-main", "type": "commit" }
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/git/commits/commit-main"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "commit-main",
            "tree": { "sha": "tree-main" }
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/git/trees/tree-main")))
        .and(query_param("recursive", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "tree-main",
            "truncated": false,
            "tree": tree_entries
        })))
        .mount(server)
        .await;
}

async fn mock_blob(server: &MockServer, repo: &str, sha: &str, bytes: &[u8]) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/git/blobs/{sha}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
            "encoding": "base64"
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn plan_selects_only_requested_category() {
    let server = MockServer::start().await;
    // Empty-ish tree without the managed file: a create is planned.
    mock_default_tree(
        &server,
        "my-repo",
        json!([{ "path": "README.md", "mode": "100644", "type": "blob", "sha": "blob-readme", "size": 3 }]),
    )
    .await;

    let mut manifest = base_manifest();
    manifest.categories.files = Some(managed_files_category(vec![dependabot_entry(
        "version: 2\n",
    )]));
    // A second, present category that must NOT be collected when not selected.
    // If it were collected, its missing mocks would surface it as blocked.
    manifest.categories.security = Some(SecurityCategoryV2::observe_sensitive());

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = vec![test_repo("my-repo")];
    let report = unified::plan(
        &client,
        &manifest,
        &repos,
        &options(vec![Category::Files], false),
    )
    .await
    .unwrap();

    assert_eq!(report.repos.len(), 1);
    let categories = &report.repos[0].categories;
    assert_eq!(categories.len(), 1, "only the selected category is planned");
    assert_eq!(categories[0].category, "files");
    assert_eq!(categories[0].status, "planned");
    assert_eq!(categories[0].actionable, 1);
    assert_eq!(report.actionable, 1);
}

#[tokio::test]
async fn plan_source_matching_files_is_zero_drift() {
    let server = MockServer::start().await;
    let content = "version: 2\n";
    mock_default_tree(
        &server,
        "my-repo",
        json!([{
            "path": ".github/dependabot.yml",
            "mode": "100644",
            "type": "blob",
            "sha": "blob-dependabot",
            "size": content.len()
        }]),
    )
    .await;
    mock_blob(&server, "my-repo", "blob-dependabot", content.as_bytes()).await;

    let mut manifest = base_manifest();
    manifest.categories.files = Some(managed_files_category(vec![dependabot_entry(content)]));

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = vec![test_repo("my-repo")];
    let report = unified::plan(
        &client,
        &manifest,
        &repos,
        &options(vec![Category::Files], false),
    )
    .await
    .unwrap();

    let files = &report.repos[0].categories[0];
    assert_eq!(files.category, "files");
    assert_eq!(files.actionable, 0, "matching source content is zero drift");
    assert_eq!(files.status, "noop");
    assert_eq!(report.actionable, 0);
}

#[tokio::test]
async fn plan_observe_files_category_yields_zero_actions() {
    let server = MockServer::start().await;
    mock_default_tree(
        &server,
        "my-repo",
        json!([{ "path": "README.md", "mode": "100644", "type": "blob", "sha": "blob-readme", "size": 3 }]),
    )
    .await;

    let mut manifest = base_manifest();
    let mut files = managed_files_category(vec![dependabot_entry("version: 2\n")]);
    files.policy = CategoryPolicy::observe();
    manifest.categories.files = Some(files);

    let client = Client::new_for_test("test-org", &server.uri());
    let repos = vec![test_repo("my-repo")];
    let report = unified::plan(
        &client,
        &manifest,
        &repos,
        &options(vec![Category::Files], false),
    )
    .await
    .unwrap();

    let files = &report.repos[0].categories[0];
    assert_eq!(files.disposition, "observe");
    assert_eq!(files.actionable, 0);
    assert_eq!(files.status, "observed");
}

#[tokio::test]
async fn apply_files_routes_through_dedicated_branch_and_pull_request() {
    let server = MockServer::start().await;
    let repo = "my-repo";
    let branch = "chore/ward-sync";

    // Plan phase: default-branch tree lacks the managed file.
    mock_default_tree(
        &server,
        repo,
        json!([{ "path": "README.md", "mode": "100644", "type": "blob", "sha": "blob-readme", "size": 3 }]),
    )
    .await;

    // Ensure dedicated branch: create ref from main head.
    Mock::given(method("POST"))
        .and(path(format!("/repos/test-org/{repo}/git/refs")))
        .and(body_partial_json(
            json!({ "ref": format!("refs/heads/{branch}") }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "ref": format!("refs/heads/{branch}"),
            "object": { "sha": "commit-main", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // Re-collect on the dedicated branch (still lacks the managed file).
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/git/ref/heads/chore/ward-sync"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": format!("refs/heads/{branch}"),
            "object": { "sha": "commit-branch", "type": "commit" }
        })))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/git/commits/commit-branch"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "commit-branch",
            "tree": { "sha": "tree-branch" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/git/trees/tree-branch"
        )))
        .and(query_param("recursive", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "tree-branch",
            "truncated": false,
            "tree": []
        })))
        .mount(&server)
        .await;

    // Atomic commit onto the dedicated branch only.
    Mock::given(method("POST"))
        .and(path(format!("/repos/test-org/{repo}/git/blobs")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "sha": "blob-new" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/repos/test-org/{repo}/git/trees")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "sha": "tree-new" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/repos/test-org/{repo}/git/commits")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "sha": "commit-new" })))
        .mount(&server)
        .await;
    // The commit ref update MUST target the dedicated branch, never main.
    Mock::given(method("PATCH"))
        .and(path(format!(
            "/repos/test-org/{repo}/git/refs/heads/chore/ward-sync"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": format!("refs/heads/{branch}"),
            "object": { "sha": "commit-new", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // Verification after the ref update sees the committed tree.
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/git/ref/heads/chore/ward-sync"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": format!("refs/heads/{branch}"),
            "object": { "sha": "commit-new", "type": "commit" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/{repo}/git/commits/commit-new"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "commit-new",
            "tree": { "sha": "tree-new" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/git/trees/tree-new")))
        .and(query_param("recursive", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "tree-new",
            "truncated": false,
            "tree": [{
                "path": ".github/dependabot.yml",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-new",
                "size": 11
            }]
        })))
        .mount(&server)
        .await;
    mock_blob(&server, repo, "blob-new", b"version: 2\n").await;

    // Pull request lookup + creation. Base must be main, head the branch.
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/{repo}/pulls")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/repos/test-org/{repo}/pulls")))
        .and(body_partial_json(json!({ "base": "main", "head": branch })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "number": 7,
            "html_url": format!("https://github.com/test-org/{repo}/pull/7"),
            "state": "open",
            "title": "chore: sync managed files",
            "head": { "ref": branch }
        })))
        .mount(&server)
        .await;

    let mut manifest = base_manifest();
    manifest.categories.files = Some(managed_files_category(vec![dependabot_entry(
        "version: 2\n",
    )]));

    let client = Client::new_for_test("test-org", &server.uri());
    let audit = ward::engine::audit_log::AuditLog::new().unwrap();
    let repos = vec![test_repo(repo)];
    let report = unified::apply(
        &client,
        &manifest,
        &repos,
        &options(vec![Category::Files], false),
        &audit,
    )
    .await
    .unwrap();

    let files = &report.repos[0].categories[0];
    assert_eq!(files.category, "files");
    assert_eq!(files.status, "success");
    assert!(files.configuration_pull_request_pending);
    assert!(
        files.details.iter().any(|d| d.contains("pull/7")),
        "a pull request should be reported, got {:?}",
        files.details
    );
    // No mock exists for PATCH refs/heads/main; if the code tried to write the
    // default branch it would have errored the category instead of succeeding.
    assert!(files.error.is_none());
}

#[tokio::test]
async fn high_impact_repository_change_is_gated() {
    async fn plan_visibility_change(allow_high_impact: bool) -> unified::UnifiedReport {
        let server = MockServer::start().await;
        // Repository currently private; desired public is a high-impact change.
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "node_id": "R_kgDOTest",
                "description": "",
                "homepage": "",
                "default_branch": "main",
                "visibility": "private",
                "archived": false,
                "is_template": false,
                "allow_forking": true,
                "has_issues": true,
                "has_projects": false,
                "has_wiki": true,
                "has_discussions": false,
                "has_pull_requests": true,
                "allow_squash_merge": true,
                "allow_merge_commit": true,
                "allow_rebase_merge": true,
                "allow_auto_merge": false,
                "delete_branch_on_merge": false,
                "allow_update_branch": false,
                "web_commit_signoff_required": false
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errors": [{ "message": "denied" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/topics"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "message": "no" })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/properties/values"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/immutable-releases"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "message": "no" })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/labels"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "no" })))
            .mount(&server)
            .await;

        let mut manifest = base_manifest();
        manifest.categories.repository = Some(RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: None,
            metadata: Some(RepositoryMetadataConfig {
                description: None,
                homepage: None,
                default_branch: None,
                visibility: Some("public".to_owned()),
                archived: None,
                is_template: None,
                allow_forking: None,
            }),
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        });

        let client = Client::new_for_test("test-org", &server.uri());
        let repos = vec![test_repo("my-repo")];
        unified::plan(
            &client,
            &manifest,
            &repos,
            &options(vec![Category::Repository], allow_high_impact),
        )
        .await
        .unwrap()
    }

    let gated = plan_visibility_change(false).await;
    let repository = &gated.repos[0].categories[0];
    assert_eq!(repository.category, "repository");
    assert_eq!(
        repository.actionable, 0,
        "visibility change is not actionable without --allow-high-impact"
    );
    assert!(
        repository.blocked >= 1,
        "high-impact visibility change is blocked, got {:?}",
        repository
    );

    let allowed = plan_visibility_change(true).await;
    let repository = &allowed.repos[0].categories[0];
    assert!(
        repository.actionable >= 1,
        "visibility change becomes actionable with --allow-high-impact"
    );
}
