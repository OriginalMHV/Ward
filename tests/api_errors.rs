#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers outside #[test] functions"
)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ward::github::Client;

const PREFIX: &str = "Could not read repo from GitHub: GET /repos/test-org/r failed with HTTP";

async fn failure(template: ResponseTemplate) -> String {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/r"))
        .respond_with(template)
        .mount(&server)
        .await;
    let client = Client::new_for_test("test-org", &server.uri());
    let err = client.get_repo("r").await.expect_err("request must fail");
    let text = format!("{err:#}");
    assert!(!text.contains("test-token"), "token leaked: {text}");
    assert!(!text.contains("body omitted"), "{text}");
    text
}

fn json_response(status: u16, body: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(body)
}

#[tokio::test]
async fn unauthorized_says_the_token_is_missing_expired_or_invalid() {
    let text = failure(json_response(401, json!({"message": "Bad credentials"}))).await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 401 Unauthorized. GitHub says: Bad credentials. The token is missing, expired or invalid. Set a valid token in GH_TOKEN or GITHUB_TOKEN, or run `gh auth login`. Then try again."
        )
    );
}

#[tokio::test]
async fn permission_names_the_access_from_the_response_header() {
    let text = failure(
        json_response(
            403,
            json!({"message": "Resource not accessible by personal access token"}),
        )
        .insert_header("x-accepted-github-permissions", "administration=read"),
    )
    .await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 403 Forbidden. GitHub says: Resource not accessible by personal access token. The token does not have the access that this call needs. It needs the fine-grained permission administration=read. Use a token with that access, or ask an organisation owner to grant it."
        )
    );
}

#[tokio::test]
async fn permission_without_a_known_endpoint_still_gives_a_next_step() {
    let text = failure(json_response(403, json!({"message": "Forbidden"}))).await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 403 Forbidden. GitHub says: Forbidden. The token does not have the access that this call needs. Use a token with that access, or ask an organisation owner to grant it."
        )
    );
}

#[tokio::test]
async fn plan_limit_explains_the_paid_plan_or_public_repository() {
    let text = failure(json_response(
        403,
        json!({"message": "Upgrade to GitHub Pro or make this repository public to enable this feature."}),
    ))
    .await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 403 Forbidden. GitHub says: Upgrade to GitHub Pro or make this repository public to enable this feature. This feature needs a paid GitHub plan or a public repository. Upgrade the plan, make the repository public, or remove this setting from the manifest."
        )
    );
}

#[tokio::test]
async fn plan_limit_is_also_recognised_on_422() {
    let text = failure(json_response(
        422,
        json!({"message": "Upgrade to GitHub Team to enable this"}),
    ))
    .await;
    assert!(
        text.contains("needs a paid GitHub plan or a public repository"),
        "{text}"
    );
}

#[tokio::test]
async fn sso_shows_the_authorization_url_from_the_header() {
    let text = failure(
        json_response(
            403,
            json!({"message": "Resource protected by organization SAML enforcement."}),
        )
        .insert_header(
            "x-github-sso",
            "required; url=https://github.com/orgs/test-org/sso?authorization_request=abc",
        ),
    )
    .await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 403 Forbidden. GitHub says: Resource protected by organization SAML enforcement. The organisation requires SAML single sign-on, and the token is not authorized for it. Authorize the token for the organisation, then try again. Authorize it here: https://github.com/orgs/test-org/sso?authorization_request=abc"
        )
    );
}

#[tokio::test]
async fn rate_limited_shows_the_reset_time() {
    let text = failure(
        json_response(429, json!({"message": "API rate limit exceeded"}))
            .insert_header("x-ratelimit-remaining", "0")
            .insert_header("x-ratelimit-reset", "4102444800"),
    )
    .await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 429 Too Many Requests. GitHub says: API rate limit exceeded. GitHub rate limit reached. The limit resets at 2100-01-01 00:00:00 UTC. Wait for the reset, then run the command again."
        )
    );
}

#[tokio::test]
async fn rate_limited_shows_retry_after() {
    let text = failure(
        json_response(
            403,
            json!({"message": "You have exceeded a secondary rate limit"}),
        )
        .insert_header("retry-after", "3600"),
    )
    .await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 403 Forbidden. GitHub says: You have exceeded a secondary rate limit. GitHub rate limit reached. GitHub asks Ward to retry in 3600 seconds. Wait for the reset, then run the command again."
        )
    );
}

#[tokio::test]
async fn not_found_says_it_may_be_hidden_from_the_token() {
    let text = failure(json_response(404, json!({"message": "Not Found"}))).await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 404 Not Found. GitHub says: Not Found. The resource was not found, or the token cannot see it. Check the name, and check that the token has access to the repository or organisation."
        )
    );
}

#[tokio::test]
async fn conflict_asks_for_a_new_read() {
    let text = failure(json_response(
        409,
        json!({"message": "Git Repository is empty"}),
    ))
    .await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 409 Conflict. GitHub says: Git Repository is empty. The request conflicts with the current state on GitHub. Someone may have changed the resource. Run the command again to read the new state."
        )
    );
}

#[tokio::test]
async fn validation_puts_each_error_on_its_own_line() {
    let text = failure(json_response(
        422,
        json!({
            "message": "Validation Failed",
            "documentation_url": "https://docs.github.com/rest/repos/repos",
            "errors": [
                {"resource": "Repository", "field": "name", "code": "invalid", "message": "name is too long"},
                {"resource": "Label", "field": "color", "code": "invalid"}
            ]
        }),
    ))
    .await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 422 Unprocessable Entity. GitHub says: Validation Failed. GitHub rejected the data that Ward sent. Fix the value below in the manifest, then try again.\n  - Repository.name (invalid): name is too long\n  - Label.color (invalid)\nDocumentation: https://docs.github.com/rest/repos/repos"
        )
    );
}

#[tokio::test]
async fn server_error_blames_github_and_says_to_retry() {
    let text = failure(json_response(500, json!({"message": "Server Error"}))).await;
    assert_eq!(
        text,
        format!(
            "{PREFIX} 500 Internal Server Error. GitHub says: Server Error. This is a problem on the GitHub side, not in your setup. Wait a few minutes and try again. See https://www.githubstatus.com for incidents."
        )
    );
}

#[tokio::test]
async fn html_body_is_not_printed_and_says_so() {
    let text =
        failure(ResponseTemplate::new(502).set_body_raw("<html>secret-page</html>", "text/html"))
            .await;
    assert!(!text.contains("secret-page"), "{text}");
    assert!(text.contains("GitHub sent no readable message"), "{text}");
}

#[tokio::test]
async fn unexpected_shape_names_the_endpoint_and_the_serde_error() {
    let text = failure(json_response(200, json!({"id": "not-a-number"}))).await;
    // `failure` expects an error, so a 200 with the wrong shape must be one.
    assert!(
        text.starts_with(
            "Could not read repo from GitHub: GET /repos/test-org/r: the response from GitHub was not in the expected shape ("
        ),
        "{text}"
    );
    assert!(text.contains("missing field"), "{text}");
}

#[test]
fn no_source_file_says_failed_to_parse() {
    fn walk(dir: &Path, hits: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                if text.contains("Failed to parse") {
                    hits.push(path.display().to_string());
                }
            }
        }
    }

    let mut hits = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    assert!(
        hits.is_empty(),
        "use read_ctx or a plain-English message instead of \"Failed to parse\" in: {hits:?}"
    );
}

async fn rendered(template: ResponseTemplate) -> String {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/test-org/r"))
        .respond_with(template)
        .mount(&server)
        .await;
    let client = Client::new_for_test("test-org", &server.uri());
    let err = client.get_repo("r").await.expect_err("request must fail");
    ward::cli::render::render_error(&err, false)
}

#[tokio::test]
async fn a_401_prints_the_failure_and_its_next_step() {
    let text = rendered(json_response(401, json!({"message": "Bad credentials"}))).await;
    assert_eq!(
        text,
        "Error: Could not read repo from GitHub\n  Caused by: GET /repos/test-org/r failed with HTTP 401 Unauthorized. GitHub says: Bad credentials. The token is missing, expired or invalid. Set a valid token in GH_TOKEN or GITHUB_TOKEN, or run `gh auth login`. Then try again."
    );
}

#[tokio::test]
async fn a_422_prints_each_entry_on_its_own_indented_line() {
    let text = rendered(json_response(
        422,
        json!({
            "message": "Validation Failed",
            "documentation_url": "https://docs.github.com/rest/repos/repos",
            "errors": [
                {"resource": "Repository", "field": "name", "code": "invalid", "message": "name is too long"},
                {"resource": "Label", "field": "color", "code": "invalid"}
            ]
        }),
    ))
    .await;
    assert_eq!(
        text,
        "Error: Could not read repo from GitHub\n  Caused by: GET /repos/test-org/r failed with HTTP 422 Unprocessable Entity. GitHub says: Validation Failed. GitHub rejected the data that Ward sent. Fix the value below in the manifest, then try again.\n      - Repository.name (invalid): name is too long\n      - Label.color (invalid)\n    Documentation: https://docs.github.com/rest/repos/repos"
    );
}
