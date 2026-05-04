mod common;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;
use ward::github::commits::CommitFile;

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
