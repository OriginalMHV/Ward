mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    CategoryPolicy, FileEncoding, FilesCategoryV2, ManagedFileV2, ManagementDisposition,
};
use ward::github::Client;
use ward::reconcile::files::{
    FilesCollection, FilesIssueKind, FilesIssueSeverity, KNOWN_CONFIG_INCLUDE_GLOBS,
    MAX_MANAGED_BLOB_BYTES, ScopedRepoFile, ScopedRepoFileKind, apply_files_plan,
    collect_files_category, plan_files_category, verify_files_category,
};

fn managed_policy(prune: bool) -> CategoryPolicy {
    CategoryPolicy {
        disposition: ManagementDisposition::Managed,
        prune,
        sensitive: false,
    }
}

fn make_category(
    prune: bool,
    include: Vec<&str>,
    exclude: Vec<&str>,
    entries: Vec<ManagedFileV2>,
) -> FilesCategoryV2 {
    FilesCategoryV2 {
        policy: managed_policy(prune),
        include: include.into_iter().map(str::to_owned).collect(),
        exclude: exclude.into_iter().map(str::to_owned).collect(),
        entries,
    }
}

async fn mock_tree_resolution(
    server: &MockServer,
    tree_sha: &str,
    tree_entries: serde_json::Value,
    truncated: bool,
) {
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ref": "refs/heads/main",
            "object": { "sha": "commit-sha", "type": "commit" }
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/commits/commit-sha"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "commit-sha",
            "tree": { "sha": tree_sha }
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/test-org/my-repo/git/trees/{tree_sha}"
        )))
        .and(query_param("recursive", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": tree_sha,
            "truncated": truncated,
            "tree": tree_entries
        })))
        .mount(server)
        .await;
}

async fn mock_blob(server: &MockServer, sha: &str, bytes: &[u8]) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/my-repo/git/blobs/{sha}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
            "encoding": "base64"
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn collect_files_category_preserves_binary_and_executable_entries() {
    let server = MockServer::start().await;
    mock_tree_resolution(
        &server,
        "tree-sha",
        json!([
            {
                "path": ".github/logo.png",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-logo",
                "size": 4
            },
            {
                "path": ".github/setup.sh",
                "mode": "100755",
                "type": "blob",
                "sha": "blob-script",
                "size": 18
            }
        ]),
        false,
    )
    .await;
    mock_blob(&server, "blob-logo", &[0, 1, 2, 255]).await;
    mock_blob(&server, "blob-script", b"#!/bin/sh\necho hi\n").await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_files_category(&client, "my-repo", Some("main"), None)
        .await
        .unwrap();

    assert_eq!(
        collected.category.include,
        KNOWN_CONFIG_INCLUDE_GLOBS
            .iter()
            .map(|pattern| (*pattern).to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(collected.category.entries.len(), 2);

    let logo = collected
        .category
        .entries
        .iter()
        .find(|entry| entry.path == ".github/logo.png")
        .unwrap();
    assert_eq!(logo.encoding, FileEncoding::Base64);
    assert_eq!(logo.mode, "100644");
    assert_eq!(logo.source_sha.as_deref(), Some("blob-logo"));

    let script = collected
        .category
        .entries
        .iter()
        .find(|entry| entry.path == ".github/setup.sh")
        .unwrap();
    assert_eq!(script.encoding, FileEncoding::Utf8);
    assert_eq!(script.mode, "100755");
    assert_eq!(script.content, "#!/bin/sh\necho hi\n");
    assert_eq!(script.source_sha.as_deref(), Some("blob-script"));
}

#[tokio::test]
async fn collect_files_category_base64_encodes_utf8_with_nul_bytes() {
    let server = MockServer::start().await;
    mock_tree_resolution(
        &server,
        "tree-control",
        json!([
            {
                "path": ".github/app.env",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-control",
                "size": 11
            }
        ]),
        false,
    )
    .await;
    mock_blob(&server, "blob-control", b"hello\0world").await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_files_category(&client, "my-repo", Some("main"), None)
        .await
        .unwrap();

    let file = collected
        .category
        .entries
        .iter()
        .find(|entry| entry.path == ".github/app.env")
        .unwrap();
    assert_eq!(file.encoding, FileEncoding::Base64);
    assert_eq!(file.content, "aGVsbG8Ad29ybGQ=");
}

#[tokio::test]
async fn collect_files_category_classifies_special_and_truncated_entries() {
    let server = MockServer::start().await;
    mock_tree_resolution(
        &server,
        "tree-special",
        json!([
            {
                "path": ".github/big.bin",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-big",
                "size": MAX_MANAGED_BLOB_BYTES + 1
            },
            {
                "path": ".github/link",
                "mode": "120000",
                "type": "blob",
                "sha": "blob-link",
                "size": 12
            },
            {
                "path": ".github/lfs.dat",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-lfs",
                "size": 120
            },
            {
                "path": ".github/module",
                "mode": "160000",
                "type": "commit",
                "sha": "module-sha"
            }
        ]),
        true,
    )
    .await;
    mock_blob(
        &server,
        "blob-lfs",
        br#"version https://git-lfs.github.com/spec/v1
oid sha256:0123456789abcdef
size 42
"#,
    )
    .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_files_category(&client, "my-repo", Some("main"), None)
        .await
        .unwrap();

    assert!(collected.category.entries.is_empty());
    assert!(collected.truncated);
    assert!(
        collected
            .issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::TruncatedTree)
    );
    assert!(
        collected
            .issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::Symlink)
    );
    assert!(
        collected
            .issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::Submodule)
    );
    assert!(
        collected
            .issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::LfsPointer)
    );
    assert!(
        collected
            .issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::Oversized)
    );

    assert!(
        collected
            .scoped_files
            .iter()
            .any(|file| file.kind == ScopedRepoFileKind::LfsPointer)
    );
}

#[tokio::test]
async fn collect_files_category_handles_empty_repository_without_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "message": "Git Repository is empty."
        })))
        .mount(&server)
        .await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_files_category(&client, "my-repo", Some("main"), None)
        .await
        .unwrap();

    assert!(collected.category.entries.is_empty());
    assert!(
        collected
            .issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::EmptyRepository)
    );
    assert!(
        collected
            .coverage
            .iter()
            .any(|entry| entry.outcome == ward::config::manifest::CoverageOutcome::Unavailable)
    );
}

#[tokio::test]
async fn plan_files_category_respects_glob_precedence_and_generates_deletions() {
    let server = MockServer::start().await;
    mock_tree_resolution(
        &server,
        "tree-plan",
        json!([
            {
                "path": ".github/generated/tmp.yml",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-generated",
                "size": 5
            },
            {
                "path": ".github/obsolete.yml",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-obsolete",
                "size": 5
            },
            {
                "path": ".github/workflows/ci.yml",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-ci",
                "size": 8
            }
        ]),
        false,
    )
    .await;
    mock_blob(&server, "blob-generated", b"skip\n").await;
    mock_blob(&server, "blob-obsolete", b"gone\n").await;
    mock_blob(&server, "blob-ci", b"name: CI\n").await;

    let desired = make_category(
        true,
        vec![".github/**"],
        vec![".github/generated/**"],
        vec![ManagedFileV2 {
            path: ".github/workflows/ci.yml".to_owned(),
            content: "name: CI\n".to_owned(),
            encoding: FileEncoding::Utf8,
            mode: "100644".to_owned(),
            source_sha: Some("blob-ci".to_owned()),
        }],
    );

    let client = Client::new_for_test("test-org", &server.uri());
    let actual = collect_files_category(&client, "my-repo", Some("main"), Some(&desired))
        .await
        .unwrap();
    let plan = plan_files_category(&desired, &actual).unwrap();

    assert_eq!(plan.unchanged, vec![".github/workflows/ci.yml".to_owned()]);
    assert!(plan.upserts.is_empty());
    assert_eq!(plan.deletions.len(), 1);
    assert_eq!(plan.deletions[0].path, ".github/obsolete.yml");
    assert!(
        plan.deletions
            .iter()
            .all(|entry| entry.path != ".github/generated/tmp.yml")
    );
}

#[test]
fn plan_files_category_blocks_prune_for_unsupported_target_only_entries() {
    let desired = make_category(
        true,
        vec![".github/**"],
        Vec::new(),
        vec![ManagedFileV2 {
            path: ".github/workflows/ci.yml".to_owned(),
            content: "name: CI\n".to_owned(),
            encoding: FileEncoding::Utf8,
            mode: "100644".to_owned(),
            source_sha: None,
        }],
    );
    let actual = FilesCollection {
        category: desired.clone(),
        scoped_files: vec![ScopedRepoFile {
            path: ".github/link".to_owned(),
            mode: Some(ward::github::contents::GitEntryMode::Symlink),
            raw_mode: "120000".to_owned(),
            object_type: ward::github::contents::GitObjectType::Blob,
            sha: "blob-link".to_owned(),
            size: Some(12),
            kind: ScopedRepoFileKind::Symlink,
            bytes: None,
        }],
        issues: Vec::new(),
        coverage: Vec::new(),
        truncated: false,
    };

    let plan = plan_files_category(&desired, &actual).unwrap();

    assert!(plan.deletions.is_empty());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::Symlink
                && issue.severity == FilesIssueSeverity::Blocker)
    );
}

#[test]
fn plan_files_category_preserves_source_zero_drift_for_unsupported_entries() {
    let desired = make_category(
        true,
        vec![".github/**"],
        Vec::new(),
        vec![ManagedFileV2 {
            path: ".github/workflows/ci.yml".to_owned(),
            content: "name: CI\n".to_owned(),
            encoding: FileEncoding::Utf8,
            mode: "100644".to_owned(),
            source_sha: Some("blob-ci".to_owned()),
        }],
    );
    let actual = FilesCollection {
        category: desired.clone(),
        scoped_files: vec![ScopedRepoFile {
            path: ".github/lfs.dat".to_owned(),
            mode: Some(ward::github::contents::GitEntryMode::File),
            raw_mode: "100644".to_owned(),
            object_type: ward::github::contents::GitObjectType::Blob,
            sha: "blob-lfs".to_owned(),
            size: Some(120),
            kind: ScopedRepoFileKind::LfsPointer,
            bytes: Some(
                br#"version https://git-lfs.github.com/spec/v1
oid sha256:0123456789abcdef
size 42
"#
                .to_vec(),
            ),
        }],
        issues: Vec::new(),
        coverage: Vec::new(),
        truncated: false,
    };

    let plan = plan_files_category(&desired, &actual).unwrap();

    assert!(plan.deletions.is_empty());
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::LfsPointer
                && issue.severity == FilesIssueSeverity::Blocker)
    );
}

#[tokio::test]
async fn plan_files_category_blocks_unsafe_paths_and_invalid_modes() {
    let desired = make_category(
        false,
        vec![".github/**"],
        Vec::new(),
        vec![
            ManagedFileV2 {
                path: "../oops".to_owned(),
                content: "bad\n".to_owned(),
                encoding: FileEncoding::Utf8,
                mode: "100644".to_owned(),
                source_sha: None,
            },
            ManagedFileV2 {
                path: ".github/link".to_owned(),
                content: "bad\n".to_owned(),
                encoding: FileEncoding::Utf8,
                mode: "120000".to_owned(),
                source_sha: None,
            },
        ],
    );
    let actual = FilesCollection {
        category: desired.clone(),
        scoped_files: Vec::new(),
        issues: Vec::new(),
        coverage: Vec::new(),
        truncated: false,
    };

    let plan = plan_files_category(&desired, &actual).unwrap();

    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::UnsafePath
                && issue.severity == FilesIssueSeverity::Blocker)
    );
    assert!(
        plan.issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::UnsupportedMode
                && issue.severity == FilesIssueSeverity::Blocker)
    );
}

#[tokio::test]
async fn apply_files_plan_sends_one_atomic_commit_payload() {
    let server = MockServer::start().await;
    mock_tree_resolution(
        &server,
        "tree-apply",
        json!([
            {
                "path": ".github/obsolete.yml",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-obsolete",
                "size": 5
            }
        ]),
        false,
    )
    .await;
    mock_blob(&server, "blob-obsolete", b"gone\n").await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/blobs"))
        .and(body_partial_json(json!({
            "content": "AAEC/w==",
            "encoding": "base64"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "blob-logo-new"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/blobs"))
        .and(body_partial_json(json!({
            "content": "#!/bin/sh\necho hi\n",
            "encoding": "utf-8"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "sha": "blob-script-new"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/repos/test-org/my-repo/git/trees"))
        .and(body_partial_json(json!({
            "base_tree": "tree-apply",
            "tree": [
                {
                    "path": ".github/logo.png",
                    "mode": "100644",
                    "type": "blob",
                    "sha": "blob-logo-new"
                },
                {
                    "path": ".github/setup.sh",
                    "mode": "100755",
                    "type": "blob",
                    "sha": "blob-script-new"
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

    let desired = make_category(
        true,
        vec![".github/**"],
        Vec::new(),
        vec![
            ManagedFileV2 {
                path: ".github/logo.png".to_owned(),
                content: "AAEC/w==".to_owned(),
                encoding: FileEncoding::Base64,
                mode: "100644".to_owned(),
                source_sha: Some("blob-logo-new".to_owned()),
            },
            ManagedFileV2 {
                path: ".github/setup.sh".to_owned(),
                content: "#!/bin/sh\necho hi\n".to_owned(),
                encoding: FileEncoding::Utf8,
                mode: "100755".to_owned(),
                source_sha: Some("blob-script-new".to_owned()),
            },
        ],
    );

    let client = Client::new_for_test("test-org", &server.uri());
    let actual = collect_files_category(&client, "my-repo", Some("main"), Some(&desired))
        .await
        .unwrap();
    let plan = plan_files_category(&desired, &actual).unwrap();
    let applied = apply_files_plan(&client, "my-repo", "main", "sync files", &plan)
        .await
        .unwrap();

    assert_eq!(applied.commit_sha.as_deref(), Some("new-commit-sha"));
    assert_eq!(applied.entry_count, 3);
    assert_eq!(plan.deletions[0].path, ".github/obsolete.yml");
}

#[tokio::test]
async fn verify_files_category_is_idempotent_for_matching_target() {
    let server = MockServer::start().await;
    mock_tree_resolution(
        &server,
        "tree-verify",
        json!([
            {
                "path": ".github/logo.png",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-logo",
                "size": 4
            },
            {
                "path": ".github/setup.sh",
                "mode": "100755",
                "type": "blob",
                "sha": "blob-script",
                "size": 18
            }
        ]),
        false,
    )
    .await;
    mock_blob(&server, "blob-logo", &[0, 1, 2, 255]).await;
    mock_blob(&server, "blob-script", b"#!/bin/sh\necho hi\n").await;

    let desired = make_category(
        false,
        vec![".github/**"],
        Vec::new(),
        vec![
            ManagedFileV2 {
                path: ".github/logo.png".to_owned(),
                content: "AAEC/w==".to_owned(),
                encoding: FileEncoding::Base64,
                mode: "100644".to_owned(),
                source_sha: Some("blob-logo".to_owned()),
            },
            ManagedFileV2 {
                path: ".github/setup.sh".to_owned(),
                content: "#!/bin/sh\necho hi\n".to_owned(),
                encoding: FileEncoding::Utf8,
                mode: "100755".to_owned(),
                source_sha: Some("blob-script".to_owned()),
            },
        ],
    );

    let client = Client::new_for_test("test-org", &server.uri());
    let verification = verify_files_category(&client, "my-repo", Some("main"), &desired)
        .await
        .unwrap();

    assert!(verification.matches);
    assert!(verification.plan.atomic_entries.is_empty());
    assert!(verification.plan.upserts.is_empty());
    assert!(verification.plan.deletions.is_empty());
}

#[tokio::test]
async fn verify_files_category_blocks_prune_when_tree_is_truncated() {
    let server = MockServer::start().await;
    mock_tree_resolution(
        &server,
        "tree-truncated",
        json!([
            {
                "path": ".github/workflows/ci.yml",
                "mode": "100644",
                "type": "blob",
                "sha": "blob-ci",
                "size": 8
            }
        ]),
        true,
    )
    .await;
    mock_blob(&server, "blob-ci", b"name: CI\n").await;

    let desired = make_category(
        true,
        vec![".github/**"],
        Vec::new(),
        vec![ManagedFileV2 {
            path: ".github/workflows/ci.yml".to_owned(),
            content: "name: CI\n".to_owned(),
            encoding: FileEncoding::Utf8,
            mode: "100644".to_owned(),
            source_sha: Some("blob-ci".to_owned()),
        }],
    );

    let client = Client::new_for_test("test-org", &server.uri());
    let verification = verify_files_category(&client, "my-repo", Some("main"), &desired)
        .await
        .unwrap();

    assert!(!verification.matches);
    assert!(
        verification
            .plan
            .issues
            .iter()
            .any(|issue| issue.kind == FilesIssueKind::TruncatedTree
                && issue.severity == FilesIssueSeverity::Blocker)
    );
}

#[test]
fn reconcile_public_types_support_manual_snapshots() {
    let snapshot = FilesCollection {
        category: make_category(false, vec![".github/**"], Vec::new(), Vec::new()),
        scoped_files: vec![ScopedRepoFile {
            path: ".github/workflows/ci.yml".to_owned(),
            mode: Some(ward::github::contents::GitEntryMode::File),
            raw_mode: "100644".to_owned(),
            object_type: ward::github::contents::GitObjectType::Blob,
            sha: "blob-ci".to_owned(),
            size: Some(8),
            kind: ScopedRepoFileKind::Managed,
            bytes: Some(b"name: CI\n".to_vec()),
        }],
        issues: Vec::new(),
        coverage: Vec::new(),
        truncated: false,
    };

    assert_eq!(snapshot.scoped_files.len(), 1);
}
