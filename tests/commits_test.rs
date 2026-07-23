mod common;

use clap::Parser;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::cli::commit::CommitCommand;
use ward::cli::{Cli, Command};
use ward::config::Manifest;
use ward::config::manifest::{
    CategoryPolicy, FileEncoding, FilesCategoryV2, ManagedFileV2, ManifestCategories, SystemConfig,
};
use ward::github::Client;
use ward::github::commits::{
    AtomicCommitEntry, AtomicCommitFile, CommitContent, CommitFile, DeleteTreeEntry,
};
use ward::github::contents::{GitEntryMode, GitObjectType};

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
async fn test_create_atomic_commit_supports_binary_bytes_and_delete_entries() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "abc123", "type": "commit" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/commits/abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "abc123",
            "tree": { "sha": "tree-sha-000" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/blobs"))
        .and(body_partial_json(json!({
            "content": "AAEC/w==",
            "encoding": "base64"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "blob-sha-001"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/trees"))
        .and(body_partial_json(json!({
            "base_tree": "tree-sha-000",
            "tree": [
                {
                    "path": "bin/run.sh",
                    "mode": "100755",
                    "type": "blob",
                    "sha": "blob-sha-001"
                },
                {
                    "path": ".github/obsolete.yml",
                    "mode": "100644",
                    "type": "blob",
                    "sha": null
                }
            ]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "new-tree-sha"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/commits"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "new-commit-sha"
        })))
        .mount(&server)
        .await;

    Mock::given(method("PATCH"))
        .and(path("/repos/test-org/my-repo/git/refs/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "new-commit-sha" }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let entries = vec![
        AtomicCommitEntry::Upsert(AtomicCommitFile {
            path: "bin/run.sh".to_owned(),
            mode: GitEntryMode::Executable,
            content: CommitContent::Bytes(vec![0, 1, 2, 255]),
        }),
        AtomicCommitEntry::Delete(DeleteTreeEntry {
            path: ".github/obsolete.yml".to_owned(),
            mode: GitEntryMode::File,
            object_type: GitObjectType::Blob,
        }),
    ];

    let sha = client
        .create_atomic_commit("my-repo", "main", "test commit", &entries)
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

#[tokio::test]
async fn test_create_branch_encodes_unicode_ref_and_accepts_existing_branch_only_if_lookup_succeeds()
 {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "source-sha-123", "type": "commit" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/refs"))
        .and(body_partial_json(json!({
            "ref": "refs/heads/feature/☃ branch",
            "sha": "source-sha-123"
        })))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Reference already exists"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/git/ref/heads/feature/%E2%98%83%20branch",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/feature/☃ branch",
            "object": { "sha": "source-sha-123", "type": "commit" }
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    client
        .create_branch("my-repo", "feature/☃ branch", "main")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_create_branch_surfaces_non_existing_422_validation_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "source-sha-123", "type": "commit" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/refs"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Validation Failed"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/feature"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let error = client
        .create_branch("my-repo", "feature", "main")
        .await
        .unwrap_err();

    assert!(error.to_string().contains("Validation Failed"));
}

#[tokio::test]
async fn test_ensure_dedicated_branch_rejects_source_branch() {
    let client = Client::new_for_test("test-org", "https://example.invalid");
    let error = client
        .ensure_dedicated_branch("my-repo", "main", "main")
        .await
        .unwrap_err();

    assert!(error.to_string().contains("dedicated branch"));
}

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
async fn test_get_file_encodes_unicode_path_and_branch_query() {
    let server = MockServer::start().await;
    let content_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "encoded");

    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/contents/.github/%E2%98%83%20config.yml",
        ))
        .and(query_param("ref", "feature/ü branch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "☃ config.yml",
            "path": ".github/☃ config.yml",
            "sha": "file-sha-encoded",
            "content": content_b64,
            "encoding": "base64"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let file = client
        .get_file("my-repo", ".github/☃ config.yml", Some("feature/ü branch"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(file.sha, "file-sha-encoded");
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

#[tokio::test]
async fn test_list_git_tree_recursive_and_get_blob_bytes() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "abc123", "type": "commit" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/commits/abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "abc123",
            "tree": { "sha": "tree-sha-000" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/trees/tree-sha-000"))
        .and(query_param("recursive", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "tree-sha-000",
            "truncated": false,
            "tree": [
                {
                    "path": "README.md",
                    "mode": "100644",
                    "type": "blob",
                    "sha": "blob-sha-001",
                    "size": 4
                },
                {
                    "path": "bin/run.sh",
                    "mode": "100755",
                    "type": "blob",
                    "sha": "blob-sha-002",
                    "size": 11
                },
                {
                    "path": "link",
                    "mode": "120000",
                    "type": "blob",
                    "sha": "blob-sha-003",
                    "size": 12
                },
                {
                    "path": "vendor/submodule",
                    "mode": "160000",
                    "type": "commit",
                    "sha": "submodule-sha"
                },
                {
                    "path": ".github",
                    "mode": "040000",
                    "type": "tree",
                    "sha": "dir-sha"
                }
            ]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/blobs/blob-sha-001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": "AAEC\n/w==",
            "encoding": "base64"
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let tree = client
        .list_git_tree_recursive("my-repo", Some("main"))
        .await
        .unwrap()
        .expect("tree should exist");

    assert_eq!(tree.sha, "tree-sha-000");
    assert!(!tree.truncated);
    assert_eq!(tree.entries.first().unwrap().path, ".github");

    let readme = tree
        .entries
        .iter()
        .find(|entry| entry.path == "README.md")
        .unwrap();
    assert_eq!(readme.mode, Some(GitEntryMode::File));
    assert_eq!(readme.object_type, GitObjectType::Blob);
    assert_eq!(readme.sha, "blob-sha-001");
    assert_eq!(readme.size, Some(4));

    let symlink = tree
        .entries
        .iter()
        .find(|entry| entry.path == "link")
        .unwrap();
    assert_eq!(symlink.mode, Some(GitEntryMode::Symlink));
    assert_eq!(symlink.object_type, GitObjectType::Blob);

    let submodule = tree
        .entries
        .iter()
        .find(|entry| entry.path == "vendor/submodule")
        .unwrap();
    assert_eq!(submodule.mode, Some(GitEntryMode::Submodule));
    assert_eq!(submodule.object_type, GitObjectType::Commit);

    let bytes = client.get_blob_bytes("my-repo", &readme.sha).await.unwrap();
    assert_eq!(bytes, vec![0, 1, 2, 255]);
}

// ---------------------------------------------------------------------------
// Hardened dedicated-branch reuse
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ensure_dedicated_branch_force_refreshes_stale_branch_without_pull_request() {
    let server = MockServer::start().await;

    // Current default head.
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "commit-main", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // The dedicated branch already exists: creating the ref is rejected.
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/refs"))
        .and(body_partial_json(
            json!({ "ref": "refs/heads/chore/ward-sync", "sha": "commit-main" }),
        ))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Reference already exists"
        })))
        .mount(&server)
        .await;

    // Existence confirmation returns a stale head ahead of the default branch.
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/git/ref/heads/chore/ward-sync",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/chore/ward-sync",
            "object": { "sha": "commit-stale", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // No open pull request tracks the branch.
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/pulls"))
        .and(query_param("state", "open"))
        .and(query_param("head", "test-org:chore/ward-sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    // The stale branch is force-reset to the current default head. There is no
    // PATCH mock for refs/heads/main, so a reset of the default branch would
    // fail the test instead of passing.
    Mock::given(method("PATCH"))
        .and(path(
            "/repos/test-org/my-repo/git/refs/heads/chore/ward-sync",
        ))
        .and(body_partial_json(
            json!({ "sha": "commit-main", "force": true }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/chore/ward-sync",
            "object": { "sha": "commit-main", "type": "commit" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let branch = client
        .ensure_dedicated_branch("my-repo", "chore/ward-sync", "main")
        .await
        .unwrap();

    assert_eq!(branch, "chore/ward-sync");
}

#[tokio::test]
async fn ensure_dedicated_branch_preserves_branch_with_open_pull_request() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "commit-main", "type": "commit" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/refs"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Reference already exists"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/git/ref/heads/chore/ward-sync",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/chore/ward-sync",
            "object": { "sha": "commit-pr", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // An open pull request already tracks the branch.
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/pulls"))
        .and(query_param("state", "open"))
        .and(query_param("head", "test-org:chore/ward-sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "number": 5,
            "html_url": "https://github.com/test-org/my-repo/pull/5",
            "state": "open",
            "title": "chore: sync managed files",
            "head": { "ref": "chore/ward-sync" }
        }])))
        .mount(&server)
        .await;

    // No PATCH mock is mounted: the branch must be preserved untouched so the
    // rerun updates the existing PR. Any force reset would 404 and error.
    let client = Client::new_for_test("test-org", &server.uri());
    let branch = client
        .ensure_dedicated_branch("my-repo", "chore/ward-sync", "main")
        .await
        .unwrap();

    assert_eq!(branch, "chore/ward-sync");
}

#[tokio::test]
async fn ensure_dedicated_branch_encodes_refs_when_refreshing_stale_unicode_branch() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "commit-main", "type": "commit" }
        })))
        .mount(&server)
        .await;

    // The create request carries the raw, unencoded ref name in its body.
    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/refs"))
        .and(body_partial_json(
            json!({ "ref": "refs/heads/feature/☃ sync" }),
        ))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "Reference already exists"
        })))
        .mount(&server)
        .await;

    // Ref lookups must percent-encode the unicode ref path.
    Mock::given(method("GET"))
        .and(path(
            "/repos/test-org/my-repo/git/ref/heads/feature/%E2%98%83%20sync",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/feature/☃ sync",
            "object": { "sha": "commit-stale", "type": "commit" }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/pulls"))
        .and(query_param("state", "open"))
        .and(query_param("head", "test-org:feature/☃ sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    // The force refresh PATCH must target the percent-encoded ref path.
    Mock::given(method("PATCH"))
        .and(path(
            "/repos/test-org/my-repo/git/refs/heads/feature/%E2%98%83%20sync",
        ))
        .and(body_partial_json(
            json!({ "sha": "commit-main", "force": true }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/feature/☃ sync",
            "object": { "sha": "commit-main", "type": "commit" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let branch = client
        .ensure_dedicated_branch("my-repo", "feature/☃ sync", "main")
        .await
        .unwrap();

    assert_eq!(branch, "feature/☃ sync");
}

// ---------------------------------------------------------------------------
// commit apply: canonical files and aggregated failures
// ---------------------------------------------------------------------------

fn parse_commit_command(args: &[&str]) -> (CommitCommand, Option<String>, Option<String>) {
    let cli = Cli::parse_from(args);
    let system = cli.system.clone();
    let repo = cli.repo.clone();
    let Command::Commit(command) = cli.command else {
        panic!("expected commit command");
    };
    (command, system, repo)
}

#[tokio::test]
async fn commit_apply_reports_collection_failures_for_every_repository() {
    let server = MockServer::start().await;

    for repo in ["repo-a", "repo-b"] {
        // Repository resolution.
        Mock::given(method("GET"))
            .and(path(format!("/repos/test-org/{repo}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(common::make_repo_json(repo, false)),
            )
            .mount(&server)
            .await;

        // The ref exists, but the intentionally unmocked commit lookup blocks collection.
        Mock::given(method("GET"))
            .and(path(format!("/repos/test-org/{repo}/git/ref/heads/main")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ref": "refs/heads/main",
                "object": { "sha": "commit-main", "type": "commit" }
            })))
            .mount(&server)
            .await;
    }

    let mut manifest = Manifest::default();
    manifest.org.name = "test-org".to_owned();
    manifest.file_delivery.branch = "chore/ward-sync".to_owned();
    manifest.categories.files = Some(FilesCategoryV2 {
        policy: CategoryPolicy::managed(),
        include: Vec::new(),
        exclude: Vec::new(),
        entries: vec![ManagedFileV2 {
            path: ".github/managed.yml".to_owned(),
            content: "version: 1\n".to_owned(),
            encoding: FileEncoding::Utf8,
            mode: "100644".to_owned(),
            source_sha: None,
        }],
    });
    manifest.systems = vec![SystemConfig {
        id: "sys".to_owned(),
        name: "System".to_owned(),
        match_prefix: false,
        exclude: Vec::new(),
        repos: vec!["repo-a".to_owned(), "repo-b".to_owned()],
        categories: ManifestCategories::default(),
    }];

    let (command, system, repo) =
        parse_commit_command(&["ward", "commit", "apply", "--yes", "--system", "sys"]);

    let client = Client::new_for_test("test-org", &server.uri());
    let result = command
        .run(&client, &manifest, system.as_deref(), repo.as_deref())
        .await;

    assert!(
        result.is_err(),
        "apply must return a non-zero error when repositories fail"
    );
    let message = format!("{}", result.unwrap_err());
    assert!(
        message.contains("2 blocked category result"),
        "error should aggregate every failure, got: {message}"
    );
}
