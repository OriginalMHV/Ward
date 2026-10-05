use std::time::{Duration, Instant};

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::config::manifest::{
    CategoryPolicy, FileEncoding, FilesCategoryV2, ManagedFileV2, ManagementDisposition,
};
use ward::github::Client;
use ward::reconcile::files::{collect_files_category, verify_files_category};

const LFS: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 42\n";

async fn mount_tree(server: &MockServer, entries: serde_json::Value) {
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
            "tree": { "sha": "tree-sha" }
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/my-repo/git/trees/tree-sha"))
        .and(query_param("recursive", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "tree-sha",
            "truncated": false,
            "tree": entries
        })))
        .mount(server)
        .await;
}

async fn mount_blob(server: &MockServer, sha: &str, bytes: &[u8], delay: Duration) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/test-org/my-repo/git/blobs/{sha}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_json(json!({
                    "content": base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        bytes
                    ),
                    "encoding": "base64"
                })),
        )
        .mount(server)
        .await;
}

fn blob_entry(name: &str) -> serde_json::Value {
    json!({
        "path": format!(".github/{name}"),
        "mode": "100644",
        "type": "blob",
        "sha": format!("blob-{name}"),
        "size": 8
    })
}

#[tokio::test]
async fn collect_fetches_blobs_concurrently_and_keeps_order() {
    let server = MockServer::start().await;
    let names: Vec<String> = (0..8).map(|i| format!("f{i}.txt")).collect();
    mount_tree(
        &server,
        serde_json::Value::Array(names.iter().map(|name| blob_entry(name)).collect()),
    )
    .await;
    for name in &names {
        mount_blob(
            &server,
            &format!("blob-{name}"),
            name.as_bytes(),
            Duration::from_millis(300),
        )
        .await;
    }

    let client = Client::new_for_test("test-org", &server.uri());
    let started = Instant::now();
    let collected = collect_files_category(&client, "my-repo", Some("main"), None)
        .await
        .unwrap();

    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "blob fetches ran serially: {:?}",
        started.elapsed()
    );
    let paths: Vec<_> = collected
        .category
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    let expected: Vec<_> = names.iter().map(|name| format!(".github/{name}")).collect();
    assert_eq!(paths, expected);
    assert_eq!(collected.category.entries[3].content, "f3.txt");
}

#[tokio::test]
async fn collect_keeps_issue_order_across_early_and_fetched_entries() {
    let server = MockServer::start().await;
    mount_tree(
        &server,
        json!([
            blob_entry("a-lfs"),
            { "path": ".github/b-link", "mode": "120000", "type": "blob", "sha": "blob-b", "size": 4 },
            blob_entry("c-lfs"),
        ]),
    )
    .await;
    mount_blob(&server, "blob-a-lfs", LFS.as_bytes(), Duration::ZERO).await;
    mount_blob(&server, "blob-c-lfs", LFS.as_bytes(), Duration::ZERO).await;

    let client = Client::new_for_test("test-org", &server.uri());
    let collected = collect_files_category(&client, "my-repo", Some("main"), None)
        .await
        .unwrap();

    let paths: Vec<_> = collected
        .issues
        .iter()
        .filter_map(|issue| issue.path.as_deref())
        .collect();
    assert_eq!(paths, [".github/a-lfs", ".github/b-link", ".github/c-lfs"]);
}

#[tokio::test]
async fn verify_fetches_blobs_concurrently() {
    let server = MockServer::start().await;
    let names: Vec<String> = (0..8).map(|i| format!("f{i}.txt")).collect();
    mount_tree(
        &server,
        serde_json::Value::Array(names.iter().map(|name| blob_entry(name)).collect()),
    )
    .await;
    for name in &names {
        mount_blob(
            &server,
            &format!("blob-{name}"),
            name.as_bytes(),
            Duration::from_millis(300),
        )
        .await;
    }
    let desired = FilesCategoryV2 {
        policy: CategoryPolicy {
            disposition: ManagementDisposition::Managed,
            prune: false,
            sensitive: false,
        },
        include: vec![".github/**".to_owned()],
        exclude: Vec::new(),
        entries: names
            .iter()
            .map(|name| ManagedFileV2 {
                path: format!(".github/{name}"),
                content: name.clone(),
                encoding: FileEncoding::Utf8,
                mode: "100644".to_owned(),
                source_sha: None,
            })
            .collect(),
    };

    let client = Client::new_for_test("test-org", &server.uri());
    let started = Instant::now();
    let result = verify_files_category(&client, "my-repo", Some("main"), &desired)
        .await
        .unwrap();

    assert!(result.matches);
    assert!(started.elapsed() < Duration::from_millis(1500));
}
