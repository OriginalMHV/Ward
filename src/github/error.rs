//! Turns a failed GitHub API response into one error with a clear next step.
//!
//! [`ApiFailure`] keeps the method, the path and the GitHub `message`. It adds a
//! class that says what went wrong and what to do. Callers that need the class
//! use `error.downcast_ref::<ApiFailure>()`. The raw response body is never
//! printed. Only the parsed `message` and `errors[]` entries are shown, and
//! the request headers (which hold the token) are never read into the error.

use std::{error::Error as StdError, fmt};

use chrono::{DateTime, TimeZone, Utc};
use reqwest::{StatusCode, header};
use serde::Deserialize;

/// The coarse HTTP outcome. The coverage mapping in the reconcile code depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHubApiErrorKind {
    NotFound,
    Forbidden,
    Unprocessable,
    UnexpectedStatus,
    Graphql,
}

/// What went wrong, in terms the user can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApiFailureClass {
    /// 401. The token is missing, expired or invalid.
    Unauthorized,
    /// 403 without a rate limit. `needed` names the access that GitHub or the endpoint shows.
    Permission { needed: Option<String> },
    /// 403 or 422 that says the feature needs a paid plan or a public repository.
    PlanLimit,
    /// The `x-github-sso` header is present.
    Sso { url: Option<String> },
    /// 403 or 429 with an exhausted limit or `retry-after`.
    RateLimited {
        reset_at: Option<DateTime<Utc>>,
        retry_after: Option<String>,
    },
    /// 404.
    NotFound,
    /// 409.
    Conflict,
    /// 422.
    Validation,
    /// 5xx.
    Server,
    /// GraphQL returned `errors`.
    Graphql,
    /// Any other status.
    Other,
}

#[derive(Debug, Clone)]
pub(crate) struct ApiFailure {
    kind: GitHubApiErrorKind,
    class: ApiFailureClass,
    status: Option<StatusCode>,
    method: String,
    path: String,
    message: Option<String>,
    details: Vec<String>,
    documentation_url: Option<String>,
    content_type: Option<String>,
    unreadable_body: bool,
}

impl ApiFailure {
    pub(crate) async fn from_response(
        response: reqwest::Response,
        method: &str,
        path: &str,
        kind: GitHubApiErrorKind,
    ) -> Self {
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();
        Self::from_parts(status, &headers, &body, method, path, kind, Utc::now())
    }

    pub(crate) fn from_parts(
        status: StatusCode,
        headers: &header::HeaderMap,
        body: &str,
        method: &str,
        path: &str,
        kind: GitHubApiErrorKind,
        now: DateTime<Utc>,
    ) -> Self {
        let payload = serde_json::from_str::<GitHubErrorPayload>(body).ok();
        let message = payload
            .as_ref()
            .and_then(|payload| payload.message.as_deref())
            .map(truncate_detail)
            .filter(|message| !message.is_empty());
        let details = payload
            .as_ref()
            .map_or_else(Vec::new, GitHubErrorPayload::safe_details);
        let class = classify(status, headers, message.as_deref(), method, path, now);
        let unreadable_body = !body.trim().is_empty() && message.is_none() && details.is_empty();

        Self {
            kind,
            class,
            status: Some(status),
            method: method.to_owned(),
            path: path.to_owned(),
            message,
            details,
            documentation_url: payload.and_then(|payload| payload.documentation_url),
            content_type: headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(truncate_detail),
            unreadable_body,
        }
    }

    pub(crate) fn graphql(path: &str, messages: &[String]) -> Self {
        Self {
            kind: GitHubApiErrorKind::Graphql,
            class: ApiFailureClass::Graphql,
            status: Some(StatusCode::OK),
            method: "POST".to_owned(),
            path: path.to_owned(),
            message: None,
            details: messages.iter().map(|m| truncate_detail(m)).collect(),
            documentation_url: None,
            content_type: None,
            unreadable_body: false,
        }
    }

    pub(crate) fn kind(&self) -> GitHubApiErrorKind {
        self.kind
    }

    #[cfg(test)]
    pub(crate) fn class(&self) -> &ApiFailureClass {
        &self.class
    }

    pub(crate) fn status(&self) -> Option<StatusCode> {
        self.status
    }
}

fn classify(
    status: StatusCode,
    headers: &header::HeaderMap,
    message: Option<&str>,
    method: &str,
    path: &str,
    now: DateTime<Utc>,
) -> ApiFailureClass {
    let lowered = message.map(str::to_ascii_lowercase).unwrap_or_default();

    if let Some(value) = header_text(headers, "x-github-sso") {
        return ApiFailureClass::Sso {
            url: sso_url(&value),
        };
    }

    let limited_status = matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    );
    if limited_status {
        let retry_after = headers
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(|value| retry_after_text(value.trim(), now));
        let exhausted = header_text(headers, "x-ratelimit-remaining").is_some_and(|v| v == "0");
        let named = lowered.contains("rate limit") || lowered.contains("abuse detection");
        if exhausted || retry_after.is_some() || named || status == StatusCode::TOO_MANY_REQUESTS {
            let reset_at = header_text(headers, "x-ratelimit-reset")
                .and_then(|value| value.parse::<i64>().ok())
                .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single());
            return ApiFailureClass::RateLimited {
                reset_at,
                retry_after,
            };
        }
    }

    if matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY
    ) && (lowered.contains("upgrade to github")
        || lowered.contains("make this repository public"))
    {
        return ApiFailureClass::PlanLimit;
    }

    match status {
        StatusCode::UNAUTHORIZED => ApiFailureClass::Unauthorized,
        StatusCode::FORBIDDEN => ApiFailureClass::Permission {
            needed: needed_access(headers, method, path),
        },
        StatusCode::NOT_FOUND => ApiFailureClass::NotFound,
        StatusCode::CONFLICT => ApiFailureClass::Conflict,
        StatusCode::UNPROCESSABLE_ENTITY => ApiFailureClass::Validation,
        status if status.is_server_error() => ApiFailureClass::Server,
        _ => ApiFailureClass::Other,
    }
}

fn header_text(headers: &header::HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?.trim();
    (!value.is_empty()).then(|| truncate_detail(value))
}

/// `x-github-sso: required; url=https://github.com/orgs/acme/sso?authorization_request=...`
fn sso_url(value: &str) -> Option<String> {
    let url = value
        .split(';')
        .filter_map(|part| part.trim().strip_prefix("url="))
        .next()?
        .trim();
    url.starts_with("https://").then(|| url.to_owned())
}

fn retry_after_text(value: &str, now: DateTime<Utc>) -> String {
    if let Ok(seconds) = value.parse::<u64>() {
        let unit = if seconds == 1 { "second" } else { "seconds" };
        return format!("in {seconds} {unit}");
    }
    match DateTime::parse_from_rfc2822(value) {
        Ok(at) => {
            let at = at.with_timezone(&Utc);
            if at <= now {
                "now".to_owned()
            } else {
                format!("after {}", format_time(at))
            }
        }
        Err(_) => "later".to_owned(),
    }
}

fn format_time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// The access that GitHub names in its response headers, or that the endpoint is known to need.
fn needed_access(headers: &header::HeaderMap, method: &str, path: &str) -> Option<String> {
    if let Some(value) = header_text(headers, "x-accepted-github-permissions") {
        return Some(format!(
            "the fine-grained permission {}",
            value.replace(';', " or")
        ));
    }
    if let Some(value) = header_text(headers, "x-accepted-oauth-scopes") {
        return Some(format!("one of the token scopes: {value}"));
    }
    endpoint_permission(method, path)
}

fn endpoint_permission(method: &str, path: &str) -> Option<String> {
    const TABLE: &[(&str, &str)] = &[
        ("/actions/secrets", "Secrets"),
        ("/actions/variables", "Variables"),
        ("/actions/", "Actions"),
        ("/environments", "Administration"),
        ("/rulesets", "Administration"),
        ("/protection", "Administration"),
        ("/collaborators", "Administration"),
        ("/invitations", "Administration"),
        ("/teams", "Administration"),
        ("/hooks", "Webhooks"),
        ("/keys", "Administration"),
        ("/autolinks", "Administration"),
        ("/code-scanning", "Code scanning alerts"),
        ("/dependabot", "Dependabot alerts"),
        ("/secret-scanning", "Secret scanning alerts"),
        ("/contents", "Contents"),
        ("/git/", "Contents"),
        ("/pulls", "Pull requests"),
    ];
    let path = path.split('?').next().unwrap_or(path);
    let (_, name) = TABLE.iter().find(|(needle, _)| path.contains(needle))?;
    let level = if matches!(method, "GET" | "HEAD") {
        "read"
    } else {
        "write"
    };
    Some(format!(
        "the repository permission \"{name}\" ({level}), or an equivalent scope"
    ))
}

impl fmt::Display for ApiFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.class == ApiFailureClass::Graphql {
            write!(f, "{} {} returned errors.", self.method, self.path)?;
        } else {
            write!(f, "{} {} failed", self.method, self.path)?;
            if let Some(status) = self.status {
                write!(f, " with HTTP {status}")?;
            }
            f.write_str(".")?;
        }

        if let Some(message) = &self.message {
            write!(f, " GitHub says: {message}.")?;
        }

        match &self.class {
            ApiFailureClass::Unauthorized => f.write_str(
                " The token is missing, expired or invalid. Set a valid token in GH_TOKEN or GITHUB_TOKEN, or run `gh auth login`. Then try again.",
            )?,
            ApiFailureClass::Permission { needed } => {
                f.write_str(" The token does not have the access that this call needs.")?;
                if let Some(needed) = needed {
                    write!(f, " It needs {needed}.")?;
                }
                f.write_str(
                    " Use a token with that access, or ask an organisation owner to grant it.",
                )?;
            }
            ApiFailureClass::PlanLimit => f.write_str(
                " This feature needs a paid GitHub plan or a public repository. Upgrade the plan, make the repository public, or remove this setting from the manifest.",
            )?,
            ApiFailureClass::Sso { url } => {
                f.write_str(
                    " The organisation requires SAML single sign-on, and the token is not authorized for it. Authorize the token for the organisation, then try again.",
                )?;
                if let Some(url) = url {
                    write!(f, " Authorize it here: {url}")?;
                }
            }
            ApiFailureClass::RateLimited {
                reset_at,
                retry_after,
            } => {
                f.write_str(" GitHub rate limit reached.")?;
                if let Some(reset_at) = reset_at {
                    write!(f, " The limit resets at {}.", format_time(*reset_at))?;
                }
                if let Some(retry_after) = retry_after {
                    write!(f, " GitHub asks Ward to retry {retry_after}.")?;
                }
                f.write_str(" Wait for the reset, then run the command again.")?;
            }
            ApiFailureClass::NotFound => f.write_str(
                " The resource was not found, or the token cannot see it. Check the name, and check that the token has access to the repository or organisation.",
            )?,
            ApiFailureClass::Conflict => f.write_str(
                " The request conflicts with the current state on GitHub. Someone may have changed the resource. Run the command again to read the new state.",
            )?,
            ApiFailureClass::Validation => {
                f.write_str(" GitHub rejected the data that Ward sent. Fix the value below in the manifest, then try again.")?;
            }
            ApiFailureClass::Server => f.write_str(
                " This is a problem on the GitHub side, not in your setup. Wait a few minutes and try again. See https://www.githubstatus.com for incidents.",
            )?,
            ApiFailureClass::Graphql => {
                f.write_str(" GitHub rejected the query for these reasons.")?;
            }
            ApiFailureClass::Other => {}
        }

        for detail in &self.details {
            write!(f, "\n  - {detail}")?;
        }

        if self.unreadable_body {
            f.write_str(" GitHub sent no readable message")?;
            if let Some(content_type) = &self.content_type {
                write!(f, " (content type {content_type})")?;
            }
            f.write_str(". Ward does not print raw response bodies.")?;
        }

        if let Some(documentation_url) = &self.documentation_url {
            write!(f, "\nDocumentation: {documentation_url}")?;
        }

        Ok(())
    }
}

impl StdError for ApiFailure {}

/// True when any error in the chain is a GitHub 404.
pub(crate) fn is_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<ApiFailure>()
            .is_some_and(|api| api.kind() == GitHubApiErrorKind::NotFound)
    })
}

/// The response was OK but its content could not be read as the expected type.
#[derive(Debug)]
pub(crate) struct ResponseShapeError {
    method: String,
    path: String,
    source: serde_json::Error,
}

impl ResponseShapeError {
    pub(crate) fn new(method: &str, path: &str, source: serde_json::Error) -> Self {
        Self {
            method: method.to_owned(),
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for ResponseShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}: the response from GitHub was not in the expected shape ({}). GitHub may have changed this endpoint. Update Ward, or report this error if it persists.",
            self.method, self.path, self.source
        )
    }
}

impl StdError for ResponseShapeError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
}

/// Adds the name of the data being read to a failed GitHub read.
pub(crate) trait ReadContext<T> {
    /// `what` names the data in plain words, for example "repository collaborators".
    fn read_ctx(self, what: &str) -> anyhow::Result<T>;
}

impl<T, E> ReadContext<T> for Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn read_ctx(self, what: &str) -> anyhow::Result<T> {
        self.map_err(|error| {
            let error: anyhow::Error = error.into();
            let typed = error.chain().any(|cause| {
                cause.downcast_ref::<ApiFailure>().is_some()
                    || cause.downcast_ref::<ResponseShapeError>().is_some()
            });
            if typed {
                error.context(format!("Could not read {what} from GitHub"))
            } else {
                error.context(format!(
                    "Could not read {what} from GitHub. The response from GitHub was not in the expected shape"
                ))
            }
        })
    }
}

#[derive(Debug, Deserialize)]
struct GitHubErrorPayload {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    documentation_url: Option<String>,
    #[serde(default)]
    errors: Vec<GitHubErrorDetail>,
}

impl GitHubErrorPayload {
    fn safe_details(&self) -> Vec<String> {
        let mut details: Vec<String> = self
            .errors
            .iter()
            .take(MAX_DETAILS)
            .map(GitHubErrorDetail::safe_summary)
            .collect();
        if self.errors.len() > MAX_DETAILS {
            details.push(format!("{} more", self.errors.len() - MAX_DETAILS));
        }
        details
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GitHubErrorDetail {
    Object {
        #[serde(default)]
        resource: Option<String>,
        #[serde(default)]
        field: Option<String>,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
    Text(String),
    Other(
        #[allow(
            dead_code,
            reason = "deserialized to accept any shape, never displayed"
        )]
        serde_json::Value,
    ),
}

/// Longest detail shown in an error. GitHub validation messages are short and are not secrets.
const MAX_DETAIL_CHARS: usize = 300;

const MAX_DETAILS: usize = 5;

fn truncate_detail(value: &str) -> String {
    // One line per detail, so GitHub text cannot inject extra lines into Ward's output.
    let flattened: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let value = flattened.split_whitespace().collect::<Vec<_>>().join(" ");
    let value = value.trim_end_matches('.');
    if value.chars().count() <= MAX_DETAIL_CHARS {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(MAX_DETAIL_CHARS).collect();
    truncated.push_str("...");
    truncated
}

impl GitHubErrorDetail {
    fn safe_summary(&self) -> String {
        match self {
            Self::Object {
                resource,
                field,
                code,
                message,
            } => {
                let mut summary = String::new();

                if let Some(resource) = resource {
                    summary.push_str(resource);
                }
                if let Some(field) = field {
                    if !summary.is_empty() {
                        summary.push('.');
                    }
                    summary.push_str(field);
                }
                if let Some(code) = code {
                    if !summary.is_empty() {
                        summary.push(' ');
                    }
                    summary.push('(');
                    summary.push_str(code);
                    summary.push(')');
                }
                if let Some(message) = message.as_deref().filter(|m| !m.trim().is_empty()) {
                    if !summary.is_empty() {
                        summary.push_str(": ");
                    }
                    summary.push_str(&truncate_detail(message));
                }

                if summary.is_empty() {
                    "additional error details omitted".to_owned()
                } else {
                    summary
                }
            }
            Self::Text(value) if !value.trim().is_empty() => truncate_detail(value),
            Self::Text(_) | Self::Other(_) => "additional error details omitted".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fail(status: u16, headers: &[(&str, &str)], body: &serde_json::Value) -> ApiFailure {
        let mut map = header::HeaderMap::new();
        for (name, value) in headers {
            map.insert(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                header::HeaderValue::from_str(value).unwrap(),
            );
        }
        ApiFailure::from_parts(
            StatusCode::from_u16(status).unwrap(),
            &map,
            &body.to_string(),
            "GET",
            "/repos/o/r",
            GitHubApiErrorKind::UnexpectedStatus,
            Utc.timestamp_opt(1_000, 0).unwrap(),
        )
    }

    #[test]
    fn plain_403_with_exhausted_header_only_is_still_rate_limited_per_spec() {
        let f = fail(
            403,
            &[("x-ratelimit-remaining", "0")],
            &json!({"message": "x"}),
        );
        assert!(matches!(f.class(), ApiFailureClass::RateLimited { .. }));
    }

    #[test]
    fn plan_limit_wins_over_validation() {
        let f = fail(
            422,
            &[],
            &json!({"message": "Upgrade to GitHub Pro or make this repository public to enable this feature."}),
        );
        assert_eq!(f.class(), &ApiFailureClass::PlanLimit);
    }

    #[test]
    fn sso_url_is_parsed_from_header() {
        let f = fail(
            403,
            &[(
                "x-github-sso",
                "required; url=https://github.com/orgs/a/sso?authorization_request=1",
            )],
            &json!({"message": "Resource protected by organization SAML enforcement."}),
        );
        assert_eq!(
            f.class(),
            &ApiFailureClass::Sso {
                url: Some("https://github.com/orgs/a/sso?authorization_request=1".to_owned())
            }
        );
    }

    #[test]
    fn read_ctx_marks_untyped_errors_as_shape_problems() {
        let err = serde_json::from_str::<u32>("\"x\"")
            .read_ctx("things")
            .unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("Could not read things from GitHub"), "{text}");
        assert!(text.contains("not in the expected shape"), "{text}");
    }

    #[test]
    fn validation_details_include_string_entries_and_truncate_long_ones() {
        let payload: GitHubErrorPayload = serde_json::from_value(json!({
            "message": "Validation Failed",
            "errors": [
                "Only organization repositories can have users and team restrictions",
                "x".repeat(1000)
            ]
        }))
        .unwrap();

        let details = payload.safe_details();

        assert_eq!(
            details[0],
            "Only organization repositories can have users and team restrictions"
        );
        assert!(details[1].len() < 400 && details[1].ends_with("..."));
    }

    #[test]
    fn validation_details_are_single_line_and_capped() {
        let payload: GitHubErrorPayload = serde_json::from_value(json!({
            "message": "Validation Failed",
            "errors": [
                "first line\nsecond line\r\n\u{1b}[31minjected",
                "b", "c", "d", "e", "f", "g"
            ]
        }))
        .unwrap();

        let details = payload.safe_details();

        assert_eq!(details[0], "first line second line [31minjected");
        assert_eq!(details.len(), MAX_DETAILS + 1);
        assert_eq!(details[MAX_DETAILS], "2 more");
    }
}
