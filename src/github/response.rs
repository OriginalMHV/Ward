use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Response, StatusCode, header};

use super::error::{ApiFailure, GitHubApiErrorKind, ResponseShapeError};
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

#[derive(Debug)]
pub(crate) enum ClassifiedResponse<T> {
    Success(T),
    NoContent,
    NotFound(ApiFailure),
    Forbidden(ApiFailure),
    Unprocessable(ApiFailure),
    Other(ApiFailure),
}

impl<T> ClassifiedResponse<T> {
    fn from_error(error: ApiFailure) -> Self {
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

async fn failure(response: Response, method: &str, path: &str) -> ApiFailure {
    let kind = kind_from_disposition(ResponseDisposition::from_status(response.status()));
    ApiFailure::from_response(response, method, path, kind).await
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
        ResponseDisposition::Success => {
            let body = response.bytes().await.with_context(|| {
                format!("{method} {path}: could not read the response from GitHub")
            })?;
            let value = serde_json::from_slice(&body)
                .map_err(|source| ResponseShapeError::new(method, path, source))?;
            Ok(ClassifiedResponse::Success(value))
        }
        ResponseDisposition::NoContent => Ok(ClassifiedResponse::NoContent),
        ResponseDisposition::NotFound
        | ResponseDisposition::Forbidden
        | ResponseDisposition::Unprocessable
        | ResponseDisposition::Other(_) => Ok(ClassifiedResponse::from_error(
            failure(response, method, path).await,
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
            failure(response, method, path).await,
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

/// Waits that apply to a retry decision.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryTiming<'a> {
    /// Exponential backoff for transient 5xx failures. The last entry is the cap.
    pub backoff_schedule: &'a [Duration],
    /// Base wait for a secondary rate limit without `retry-after`. Doubles per retry.
    pub secondary_wait: Duration,
    /// Longest rate limit wait that Ward accepts. A longer wait is not retried.
    pub max_rate_limit_wait: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryKind {
    Transient,
    PrimaryRateLimit,
    SecondaryRateLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetryPlan {
    pub delay: Duration,
    pub kind: RetryKind,
}

/// Decide whether and when to retry a response.
///
/// `rate_limit_body` is true when a 403 body says the request hit a rate limit.
/// A 403 is a rate limit only when the headers or the body say so, never because of
/// `x-ratelimit-remaining: 0` alone, since a plain permission error can carry it.
pub(crate) fn retry_delay(
    status: StatusCode,
    headers: &header::HeaderMap,
    rate_limit_body: bool,
    retry_number: usize,
    timing: &RetryTiming<'_>,
    now: DateTime<Utc>,
) -> Option<RetryPlan> {
    match status {
        StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN => {
            rate_limit_plan(status, headers, rate_limit_body, retry_number, timing, now)
        }
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => {
            fallback_retry_delay(retry_number, timing.backoff_schedule).map(|delay| RetryPlan {
                delay,
                kind: RetryKind::Transient,
            })
        }
        _ => None,
    }
}

fn rate_limit_plan(
    status: StatusCode,
    headers: &header::HeaderMap,
    rate_limit_body: bool,
    retry_number: usize,
    timing: &RetryTiming<'_>,
    now: DateTime<Utc>,
) -> Option<RetryPlan> {
    let backoff = || {
        let factor = 1u32 << retry_number.saturating_sub(1).min(16);
        timing.secondary_wait.saturating_mul(factor)
    };
    let (delay, kind) = if let Some(delay) = parse_retry_after(headers, now) {
        (delay, RetryKind::SecondaryRateLimit)
    } else if is_rate_limit_exhausted(headers)
        && (status == StatusCode::TOO_MANY_REQUESTS || rate_limit_body)
    {
        let delay = parse_rate_limit_reset(headers, now).unwrap_or_else(backoff);
        (delay, RetryKind::PrimaryRateLimit)
    } else if status == StatusCode::TOO_MANY_REQUESTS || rate_limit_body {
        (backoff(), RetryKind::SecondaryRateLimit)
    } else {
        return None;
    };

    (delay <= timing.max_rate_limit_wait).then_some(RetryPlan { delay, kind })
}

pub(crate) fn mentions_rate_limit(body: &[u8]) -> bool {
    let body = String::from_utf8_lossy(body).to_ascii_lowercase();
    body.contains("rate limit") || body.contains("abuse detection")
}

pub(crate) fn is_rate_limit_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN
    )
}

pub(crate) fn needs_body_check(headers: &header::HeaderMap) -> bool {
    !has_retry_after(headers)
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

    use super::{
        ApiFailure, ClassifiedResponse, RetryKind, RetryPlan, RetryTiming, classify_empty,
        classify_json, mentions_rate_limit, retry_delay,
    };

    #[tokio::test]
    async fn classify_json_keeps_validation_details_without_leaking_the_raw_body() {
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
        assert_eq!(error.kind(), super::GitHubApiErrorKind::Unprocessable);
        assert_ne!(error.kind(), super::GitHubApiErrorKind::NotFound);

        let display = error.to_string();
        assert!(display.contains("Validation Failed"));
        assert!(display.contains("Repository.name (invalid)"));
        assert!(!display.contains("body omitted"));
        assert!(display.contains("Repository.name (invalid): top-secret-value"));
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

        assert_eq!(error.kind(), super::GitHubApiErrorKind::Forbidden);
    }

    #[test]
    fn graphql_errors_are_collector_friendly() {
        let error = ApiFailure::graphql(
            "/graphql",
            &["Resource not accessible by integration".to_owned()],
        );

        assert_eq!(error.kind(), super::GitHubApiErrorKind::Graphql);
        assert!(
            error
                .to_string()
                .contains("Resource not accessible by integration")
        );
        assert!(!error.to_string().contains("body omitted"));
    }

    const SCHEDULE: [Duration; 3] = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
    ];
    const TIMING: RetryTiming<'static> = RetryTiming {
        backoff_schedule: &SCHEDULE,
        secondary_wait: Duration::from_secs(60),
        max_rate_limit_wait: Duration::from_secs(300),
    };

    fn plan(
        status: StatusCode,
        headers: &header::HeaderMap,
        secondary_body: bool,
        retry_number: usize,
        now: chrono::DateTime<Utc>,
    ) -> Option<RetryPlan> {
        retry_delay(status, headers, secondary_body, retry_number, &TIMING, now)
    }

    fn exhausted_headers(reset: &'static str) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "x-ratelimit-remaining",
            header::HeaderValue::from_static("0"),
        );
        headers.insert("x-ratelimit-reset", header::HeaderValue::from_static(reset));
        headers
    }

    #[test]
    fn retry_delay_respects_retry_after_for_rate_limited_forbidden() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::RETRY_AFTER, header::HeaderValue::from_static("7"));

        let result = plan(StatusCode::FORBIDDEN, &headers, false, 1, Utc::now());

        assert_eq!(
            result,
            Some(RetryPlan {
                delay: Duration::from_secs(7),
                kind: RetryKind::SecondaryRateLimit
            })
        );
    }

    #[test]
    fn retry_delay_uses_rate_limit_reset_when_remaining_is_zero() {
        let headers = exhausted_headers("1784023205");
        let now = Utc.timestamp_opt(1784023200, 0).single().unwrap();

        let result = plan(StatusCode::FORBIDDEN, &headers, true, 1, now);

        assert_eq!(
            result,
            Some(RetryPlan {
                delay: Duration::from_secs(5),
                kind: RetryKind::PrimaryRateLimit
            })
        );
        assert_eq!(
            plan(StatusCode::FORBIDDEN, &headers, false, 1, now),
            None,
            "a 403 without a rate limit message is a permission error"
        );
        assert_eq!(
            plan(StatusCode::TOO_MANY_REQUESTS, &headers, false, 1, now),
            result
        );
    }

    #[test]
    fn retry_delay_ignores_rate_limit_reset_for_server_errors() {
        let headers = exhausted_headers("1784026800");
        let now = Utc.timestamp_opt(1784023200, 0).single().unwrap();

        for status in [
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert_eq!(
                plan(status, &headers, false, 1, now),
                Some(RetryPlan {
                    delay: Duration::from_secs(1),
                    kind: RetryKind::Transient
                })
            );
        }
    }

    #[test]
    fn retry_delay_does_not_wait_for_primary_reset_beyond_cap() {
        let headers = exhausted_headers("1784026800");
        let now = Utc.timestamp_opt(1784023200, 0).single().unwrap();

        assert_eq!(plan(StatusCode::FORBIDDEN, &headers, true, 1, now), None);
    }

    #[test]
    fn retry_delay_waits_one_minute_for_secondary_limit_without_retry_after() {
        let headers = header::HeaderMap::new();

        assert_eq!(
            plan(StatusCode::FORBIDDEN, &headers, true, 1, Utc::now()),
            Some(RetryPlan {
                delay: Duration::from_secs(60),
                kind: RetryKind::SecondaryRateLimit
            })
        );
        assert_eq!(
            plan(
                StatusCode::TOO_MANY_REQUESTS,
                &headers,
                false,
                2,
                Utc::now()
            )
            .map(|plan| plan.delay),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            plan(StatusCode::FORBIDDEN, &headers, true, 4, Utc::now()),
            None
        );
    }

    #[test]
    fn secondary_rate_limit_body_detection_is_case_insensitive() {
        assert!(mentions_rate_limit(
            br#"{"message":"You have exceeded a Secondary Rate Limit."}"#
        ));
        assert!(mentions_rate_limit(
            br#"{"message":"API rate limit exceeded for user ID 1."}"#
        ));
        assert!(!mentions_rate_limit(
            br#"{"message":"Resource not accessible by integration"}"#
        ));
    }

    #[test]
    fn retry_delay_falls_back_to_bounded_backoff() {
        let headers = header::HeaderMap::new();

        for (retry, expected) in [(1, 1), (2, 2), (3, 4), (4, 4)] {
            assert_eq!(
                plan(
                    StatusCode::SERVICE_UNAVAILABLE,
                    &headers,
                    false,
                    retry,
                    Utc::now()
                )
                .map(|plan| plan.delay),
                Some(Duration::from_secs(expected))
            );
        }
    }

    #[test]
    fn retry_delay_does_not_retry_validation_or_ordinary_forbidden() {
        let headers = header::HeaderMap::new();

        assert_eq!(
            plan(
                StatusCode::UNPROCESSABLE_ENTITY,
                &headers,
                false,
                1,
                Utc::now()
            ),
            None
        );
        assert_eq!(
            plan(StatusCode::FORBIDDEN, &headers, false, 1, Utc::now()),
            None
        );
    }
}
