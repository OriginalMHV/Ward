use std::{error::Error as StdError, fmt, time::Duration};

use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Response, StatusCode, header};
use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseDisposition {
    Success,
    NoContent,
    NotFound,
    Forbidden,
    Unprocessable,
    Other(StatusCode),
}

impl ResponseDisposition {
    fn from_status(status: StatusCode) -> Self {
        match status {
            StatusCode::NO_CONTENT => Self::NoContent,
            StatusCode::NOT_FOUND => Self::NotFound,
            StatusCode::FORBIDDEN => Self::Forbidden,
            StatusCode::UNPROCESSABLE_ENTITY => Self::Unprocessable,
            status if status.is_success() => Self::Success,
            status => Self::Other(status),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHubApiErrorKind {
    NotFound,
    Forbidden,
    Unprocessable,
    UnexpectedStatus,
    Graphql,
}

#[derive(Debug, Clone)]
pub(crate) struct GitHubApiError {
    kind: GitHubApiErrorKind,
    status: Option<StatusCode>,
    method: String,
    path: String,
    message: Option<String>,
    details: Vec<String>,
    documentation_url: Option<String>,
    content_type: Option<String>,
    body_omitted: bool,
}

impl GitHubApiError {
    async fn from_response(response: Response, method: &str, path: &str) -> Self {
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let body = response.text().await.unwrap_or_default();
        let payload = serde_json::from_str::<GitHubErrorPayload>(&body).ok();

        Self {
            kind: kind_from_disposition(ResponseDisposition::from_status(status)),
            status: Some(status),
            method: method.to_owned(),
            path: path.to_owned(),
            message: payload.as_ref().and_then(|payload| payload.message.clone()),
            details: payload
                .as_ref()
                .map_or_else(Vec::new, GitHubErrorPayload::safe_details),
            documentation_url: payload.and_then(|payload| payload.documentation_url),
            content_type,
            body_omitted: !body.trim().is_empty(),
        }
    }

    pub(crate) fn graphql(path: &str, messages: Vec<String>) -> Self {
        Self {
            kind: GitHubApiErrorKind::Graphql,
            status: Some(StatusCode::OK),
            method: "POST".to_owned(),
            path: path.to_owned(),
            message: Some("GitHub GraphQL returned error(s)".to_owned()),
            details: messages,
            documentation_url: None,
            content_type: Some("application/json".to_owned()),
            body_omitted: true,
        }
    }

    pub(crate) fn kind(&self) -> GitHubApiErrorKind {
        self.kind
    }

    pub(crate) fn status(&self) -> Option<StatusCode> {
        self.status
    }

    #[allow(dead_code)]
    pub(crate) fn is_not_found(&self) -> bool {
        self.kind == GitHubApiErrorKind::NotFound
    }

    #[allow(dead_code)]
    pub(crate) fn is_forbidden(&self) -> bool {
        self.kind == GitHubApiErrorKind::Forbidden
    }

    #[allow(dead_code)]
    pub(crate) fn is_unprocessable(&self) -> bool {
        self.kind == GitHubApiErrorKind::Unprocessable
    }
}

impl fmt::Display for GitHubApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.path)?;

        if let Some(status) = self.status {
            write!(f, " failed with HTTP {status}")?;
        } else {
            write!(f, " failed")?;
        }

        if let Some(message) = &self.message {
            write!(f, ": {message}")?;
        }

        if !self.details.is_empty() {
            write!(f, " [{}]", self.details.join("; "))?;
        }

        if let Some(documentation_url) = &self.documentation_url {
            write!(f, " ({documentation_url})")?;
        }

        if self.body_omitted {
            if let Some(content_type) = &self.content_type {
                write!(f, " [response body omitted, content type {content_type}]")?;
            } else {
                write!(f, " [response body omitted]")?;
            }
        }

        Ok(())
    }
}

impl StdError for GitHubApiError {}

#[derive(Debug)]
pub(crate) enum ClassifiedResponse<T> {
    Success(T),
    NoContent,
    NotFound(GitHubApiError),
    Forbidden(GitHubApiError),
    Unprocessable(GitHubApiError),
    Other(GitHubApiError),
}

impl<T> ClassifiedResponse<T> {
    fn from_error(error: GitHubApiError) -> Self {
        match error.kind() {
            GitHubApiErrorKind::NotFound => Self::NotFound(error),
            GitHubApiErrorKind::Forbidden => Self::Forbidden(error),
            GitHubApiErrorKind::Unprocessable => Self::Unprocessable(error),
            GitHubApiErrorKind::UnexpectedStatus | GitHubApiErrorKind::Graphql => {
                Self::Other(error)
            }
        }
    }
}

pub(crate) async fn classify_json<T>(
    response: Response,
    method: &str,
    path: &str,
) -> Result<ClassifiedResponse<T>>
where
    T: DeserializeOwned,
{
    match ResponseDisposition::from_status(response.status()) {
        ResponseDisposition::Success => Ok(ClassifiedResponse::Success(response.json().await?)),
        ResponseDisposition::NoContent => Ok(ClassifiedResponse::NoContent),
        ResponseDisposition::NotFound
        | ResponseDisposition::Forbidden
        | ResponseDisposition::Unprocessable
        | ResponseDisposition::Other(_) => Ok(ClassifiedResponse::from_error(
            GitHubApiError::from_response(response, method, path).await,
        )),
    }
}

pub(crate) async fn classify_empty(
    response: Response,
    method: &str,
    path: &str,
) -> Result<ClassifiedResponse<()>> {
    match ResponseDisposition::from_status(response.status()) {
        ResponseDisposition::Success => Ok(ClassifiedResponse::Success(())),
        ResponseDisposition::NoContent => Ok(ClassifiedResponse::NoContent),
        ResponseDisposition::NotFound
        | ResponseDisposition::Forbidden
        | ResponseDisposition::Unprocessable
        | ResponseDisposition::Other(_) => Ok(ClassifiedResponse::from_error(
            GitHubApiError::from_response(response, method, path).await,
        )),
    }
}

pub(crate) async fn expect_json<T>(response: Response, method: &str, path: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    match classify_json(response, method, path).await? {
        ClassifiedResponse::Success(value) => Ok(value),
        ClassifiedResponse::NoContent => Err(anyhow!(
            "{method} {path} returned HTTP 204 No Content when JSON was expected"
        )),
        ClassifiedResponse::NotFound(error)
        | ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Err(anyhow!(error)),
    }
}

pub(crate) async fn optional_json<T>(
    response: Response,
    method: &str,
    path: &str,
) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    match classify_json(response, method, path).await? {
        ClassifiedResponse::Success(value) => Ok(Some(value)),
        ClassifiedResponse::NoContent | ClassifiedResponse::NotFound(_) => Ok(None),
        ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Err(anyhow!(error)),
    }
}

pub(crate) async fn expect_empty(response: Response, method: &str, path: &str) -> Result<()> {
    match classify_empty(response, method, path).await? {
        ClassifiedResponse::Success(()) | ClassifiedResponse::NoContent => Ok(()),
        ClassifiedResponse::NotFound(error)
        | ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Err(anyhow!(error)),
    }
}

pub(crate) fn retry_delay(
    status: StatusCode,
    headers: &header::HeaderMap,
    retry_number: usize,
    fallback_schedule: &[Duration],
    now: DateTime<Utc>,
) -> Option<Duration> {
    match status {
        StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => parse_retry_after(headers, now)
            .or_else(|| parse_rate_limit_reset(headers, now))
            .or_else(|| fallback_retry_delay(retry_number, fallback_schedule)),
        StatusCode::FORBIDDEN if has_retry_after(headers) || is_rate_limit_exhausted(headers) => {
            parse_retry_after(headers, now)
                .or_else(|| parse_rate_limit_reset(headers, now))
                .or_else(|| fallback_retry_delay(retry_number, fallback_schedule))
        }
        _ => None,
    }
}

fn kind_from_disposition(disposition: ResponseDisposition) -> GitHubApiErrorKind {
    match disposition {
        ResponseDisposition::NotFound => GitHubApiErrorKind::NotFound,
        ResponseDisposition::Forbidden => GitHubApiErrorKind::Forbidden,
        ResponseDisposition::Unprocessable => GitHubApiErrorKind::Unprocessable,
        ResponseDisposition::Success
        | ResponseDisposition::NoContent
        | ResponseDisposition::Other(_) => GitHubApiErrorKind::UnexpectedStatus,
    }
}

fn has_retry_after(headers: &header::HeaderMap) -> bool {
    headers.contains_key(header::RETRY_AFTER)
}

fn is_rate_limit_exhausted(headers: &header::HeaderMap) -> bool {
    headers
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim() == "0")
}

fn parse_retry_after(headers: &header::HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();

    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let retry_at = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    Some(duration_until(retry_at, now))
}

fn parse_rate_limit_reset(headers: &header::HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
    let timestamp = headers
        .get("x-ratelimit-reset")?
        .to_str()
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()?;
    let reset_at = Utc.timestamp_opt(timestamp, 0).single()?;
    Some(duration_until(reset_at, now))
}

fn duration_until(target: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    match (target - now).to_std() {
        Ok(duration) => duration,
        Err(_) => Duration::ZERO,
    }
}

fn fallback_retry_delay(retry_number: usize, fallback_schedule: &[Duration]) -> Option<Duration> {
    if fallback_schedule.is_empty() {
        return None;
    }

    let index = retry_number
        .saturating_sub(1)
        .min(fallback_schedule.len().saturating_sub(1));
    fallback_schedule.get(index).copied()
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
        self.errors
            .iter()
            .map(GitHubErrorDetail::safe_summary)
            .collect()
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
    },
    Text(String),
    Other(serde_json::Value),
}

impl GitHubErrorDetail {
    fn safe_summary(&self) -> String {
        match self {
            Self::Object {
                resource,
                field,
                code,
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

                if summary.is_empty() {
                    "additional error details omitted".to_owned()
                } else {
                    summary
                }
            }
            Self::Text(value) => {
                let _ = value;
                "additional error details omitted".to_owned()
            }
            Self::Other(value) => {
                let _ = value;
                "additional error details omitted".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeZone, Utc};
    use reqwest::StatusCode;
    use reqwest::header;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::github::Client;

    use super::{ClassifiedResponse, GitHubApiError, classify_empty, classify_json, retry_delay};

    #[tokio::test]
    async fn classify_json_preserves_structured_context_without_leaking_raw_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/private-repo"))
            .respond_with(ResponseTemplate::new(422).set_body_json(json!({
                "message": "Validation Failed",
                "documentation_url": "https://docs.github.com/rest/repos/repos",
                "errors": [{
                    "resource": "Repository",
                    "field": "name",
                    "code": "invalid",
                    "message": "top-secret-value"
                }],
                "secret": "do-not-log"
            })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let response = client.get("/repos/test-org/private-repo").await.unwrap();
        let classified =
            classify_json::<serde_json::Value>(response, "GET", "/repos/test-org/private-repo")
                .await
                .unwrap();

        let error = match classified {
            ClassifiedResponse::Unprocessable(error) => error,
            other => panic!("expected unprocessable response, got {other:?}"),
        };

        assert_eq!(error.status(), Some(StatusCode::UNPROCESSABLE_ENTITY));
        assert!(error.is_unprocessable());
        assert!(!error.is_not_found());

        let display = error.to_string();
        assert!(display.contains("Validation Failed"));
        assert!(display.contains("Repository.name (invalid)"));
        assert!(display.contains("response body omitted"));
        assert!(!display.contains("top-secret-value"));
        assert!(!display.contains("do-not-log"));
    }

    #[tokio::test]
    async fn classify_empty_distinguishes_no_content_and_forbidden() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/repos/test-org/my-repo/rulesets/1"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/repos/test-org/my-repo/rulesets/2"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "message": "Resource not accessible by integration"
            })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());

        let no_content = classify_empty(
            client
                .delete("/repos/test-org/my-repo/rulesets/1")
                .await
                .unwrap(),
            "DELETE",
            "/repos/test-org/my-repo/rulesets/1",
        )
        .await
        .unwrap();
        assert!(matches!(no_content, ClassifiedResponse::NoContent));

        let forbidden = classify_empty(
            client
                .delete("/repos/test-org/my-repo/rulesets/2")
                .await
                .unwrap(),
            "DELETE",
            "/repos/test-org/my-repo/rulesets/2",
        )
        .await
        .unwrap();

        let error = match forbidden {
            ClassifiedResponse::Forbidden(error) => error,
            other => panic!("expected forbidden response, got {other:?}"),
        };

        assert!(error.is_forbidden());
    }

    #[test]
    fn graphql_errors_are_collector_friendly() {
        let error = GitHubApiError::graphql(
            "/graphql",
            vec!["Resource not accessible by integration".to_owned()],
        );

        assert_eq!(error.kind(), super::GitHubApiErrorKind::Graphql);
        assert!(
            error
                .to_string()
                .contains("Resource not accessible by integration")
        );
        assert!(error.to_string().contains("response body omitted"));
    }

    #[test]
    fn retry_delay_respects_retry_after_for_rate_limited_forbidden() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::RETRY_AFTER, header::HeaderValue::from_static("7"));

        let delay = retry_delay(
            StatusCode::FORBIDDEN,
            &headers,
            1,
            &[
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
            Utc.with_ymd_and_hms(2026, 7, 14, 10, 0, 0).unwrap(),
        );

        assert_eq!(delay, Some(Duration::from_secs(7)));
    }

    #[test]
    fn retry_delay_uses_rate_limit_reset_when_remaining_is_zero() {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "x-ratelimit-remaining",
            header::HeaderValue::from_static("0"),
        );
        headers.insert(
            "x-ratelimit-reset",
            header::HeaderValue::from_static("1784023205"),
        );

        let delay = retry_delay(
            StatusCode::FORBIDDEN,
            &headers,
            1,
            &[
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
            Utc.timestamp_opt(1784023200, 0).single().unwrap(),
        );

        assert_eq!(delay, Some(Duration::from_secs(5)));
    }

    #[test]
    fn retry_delay_falls_back_to_bounded_backoff() {
        let headers = header::HeaderMap::new();
        let schedule = [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ];

        assert_eq!(
            retry_delay(
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                1,
                &schedule,
                Utc::now()
            ),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            retry_delay(
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                2,
                &schedule,
                Utc::now()
            ),
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            retry_delay(
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                3,
                &schedule,
                Utc::now()
            ),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            retry_delay(
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                4,
                &schedule,
                Utc::now()
            ),
            Some(Duration::from_secs(4))
        );
    }

    #[test]
    fn retry_delay_does_not_retry_validation_or_ordinary_forbidden() {
        let headers = header::HeaderMap::new();
        let schedule = [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ];

        assert_eq!(
            retry_delay(
                StatusCode::UNPROCESSABLE_ENTITY,
                &headers,
                1,
                &schedule,
                Utc::now()
            ),
            None
        );
        assert_eq!(
            retry_delay(StatusCode::FORBIDDEN, &headers, 1, &schedule, Utc::now()),
            None
        );
    }
}
