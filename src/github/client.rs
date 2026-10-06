use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::{OnceCell, Semaphore};

use crate::config::auth;

use super::access::CustomRepositoryRole;
use super::actions::ReadOutcome;
use super::metadata;
use super::response;
use super::rulesets::{GitHubUser, InstalledApp};
use super::security::CodeSecurityConfiguration;
use super::teams::Team;

type KeyedCells<T> = Mutex<HashMap<String, Arc<OnceCell<T>>>>;

/// Organization-level reads that are identical for every repository in a run.
///
/// Full outcomes are cached, including the `ReadOutcome` coverage variants, so every
/// repository reports the same coverage. Errors are not cached.
#[derive(Default)]
pub(crate) struct OrgLookups {
    pub(crate) teams: OnceCell<Vec<Team>>,
    pub(crate) teams_checked: OnceCell<ReadOutcome<Vec<Team>>>,
    pub(crate) custom_roles_checked: OnceCell<ReadOutcome<Vec<CustomRepositoryRole>>>,
    pub(crate) installations: OnceCell<Vec<InstalledApp>>,
    pub(crate) code_security_configurations: OnceCell<Vec<CodeSecurityConfiguration>>,
    pub(crate) team_ids: KeyedCells<u64>,
    pub(crate) users: KeyedCells<GitHubUser>,
}

/// GitHub API client with rate limiting and concurrency control.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    pub(crate) org: String,
    semaphore: Arc<Semaphore>,
    base_url: String,
    retry_policy: RetryPolicy,
    org_lookups: Option<Arc<OrgLookups>>,
}

impl Client {
    /// A client that shares the connection pool but reads organization lookups fresh.
    ///
    /// Use it after mutations, for example for post-apply verification.
    pub fn uncached(&self) -> Self {
        Self {
            org_lookups: None,
            ..self.clone()
        }
    }

    pub(crate) async fn cached_org<T, F, Fut>(&self, cell: F, fetch: Fut) -> Result<T>
    where
        T: Clone,
        F: FnOnce(&OrgLookups) -> &OnceCell<T>,
        Fut: Future<Output = Result<T>>,
    {
        match &self.org_lookups {
            Some(lookups) => cell(lookups).get_or_try_init(|| fetch).await.cloned(),
            None => fetch.await,
        }
    }

    pub(crate) async fn cached_org_keyed<T, F, Fut>(
        &self,
        cells: F,
        key: &str,
        fetch: Fut,
    ) -> Result<T>
    where
        T: Clone,
        F: FnOnce(&OrgLookups) -> &KeyedCells<T>,
        Fut: Future<Output = Result<T>>,
    {
        let Some(lookups) = &self.org_lookups else {
            return fetch.await;
        };
        let cell = {
            let mut guard = cells(lookups)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(guard.entry(key.to_owned()).or_default())
        };
        cell.get_or_try_init(|| fetch).await.cloned()
    }

    /// The GitHub organization this client targets.
    pub fn org(&self) -> &str {
        &self.org
    }

    pub fn new(org: &str, parallelism: usize) -> Result<Self> {
        validate_parallelism(parallelism)?;
        let token = auth::resolve_token()?;
        let headers = default_headers(
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("Invalid token characters")?,
        )?;

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            http,
            org: org.to_owned(),
            semaphore: Arc::new(Semaphore::new(parallelism)),
            base_url: "https://api.github.com".to_owned(),
            retry_policy: RetryPolicy::default(),
            org_lookups: Some(Arc::default()),
        })
    }

    /// Make a GET request to the GitHub API.
    pub async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("GET", &url, self.http.get(&url)).await
    }

    /// Make a PUT request to the GitHub API.
    pub async fn put(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send(
            "PUT",
            &url,
            self.http.put(&url).header(header::CONTENT_LENGTH, 0),
        )
        .await
    }

    /// Make a PATCH request with a JSON body.
    pub async fn patch_json<T: Serialize + Sync>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("PATCH", &url, self.http.patch(&url).json(body))
            .await
    }

    /// Make a POST request with a JSON body.
    pub async fn post_json<T: Serialize + Sync>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("POST", &url, self.http.post(&url).json(body))
            .await
    }

    /// Make a PUT request with a JSON body.
    pub async fn put_json<T: Serialize + Sync>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("PUT", &url, self.http.put(&url).json(body)).await
    }

    /// Make a DELETE request.
    pub async fn delete(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("DELETE", &url, self.http.delete(&url)).await
    }

    /// Make a DELETE request with a JSON body.
    pub async fn delete_json<T: Serialize + Sync>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("DELETE", &url, self.http.delete(&url).json(body))
            .await
    }

    pub async fn graphql<T, V>(&self, query: &str, variables: &V) -> Result<T>
    where
        T: DeserializeOwned,
        V: Serialize + Sync,
    {
        #[derive(Serialize)]
        struct GraphqlRequest<'a, V> {
            query: &'a str,
            variables: &'a V,
        }

        #[derive(Deserialize)]
        #[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
        struct GraphqlResponse<T> {
            data: Option<T>,
            #[serde(default)]
            errors: Vec<GraphqlError>,
        }

        #[derive(Deserialize)]
        struct GraphqlError {
            message: String,
        }

        let path = "/graphql";
        let url = format!("{}{}", self.base_url, path);
        let request = self
            .http
            .post(&url)
            .json(&GraphqlRequest { query, variables });
        let response = self
            .send_with("POST", &url, request, is_read_only_query(query))
            .await?;
        let body: GraphqlResponse<T> = response::expect_json(response, "POST", path).await?;

        if !body.errors.is_empty() {
            return Err(anyhow::Error::new(response::GitHubApiError::graphql(
                path,
                body.errors.into_iter().map(|error| error.message).collect(),
            )));
        }

        body.data
            .context("GitHub GraphQL response did not include a data payload")
    }

    #[doc(hidden)]
    #[allow(
        clippy::unwrap_used,
        reason = "test-only constructor with static inputs"
    )]
    pub fn new_for_test(org: &str, base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .default_headers(
                default_headers(HeaderValue::from_static("Bearer test-token")).unwrap(),
            )
            .build()
            .unwrap();

        Self {
            http,
            org: org.to_owned(),
            semaphore: Arc::new(Semaphore::new(10)),
            base_url: base_url.to_owned(),
            retry_policy: RetryPolicy::immediate_for_tests(),
            org_lookups: Some(Arc::default()),
        }
    }

    async fn send(
        &self,
        method: &str,
        url: &str,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        self.send_with(method, url, request, method != "POST").await
    }

    /// `retry_server_errors` must be false for requests that may have taken effect
    /// before a 5xx, because a retry could duplicate the change.
    async fn send_with(
        &self,
        method: &str,
        url: &str,
        request: reqwest::RequestBuilder,
        retry_server_errors: bool,
    ) -> Result<reqwest::Response> {
        let _permit = self.semaphore.acquire().await?;
        let mut request = request;
        let mut attempt = 1usize;
        let mut retried_after_server_error = false;

        loop {
            tracing::debug!("{method} {url} attempt {attempt}");
            let retry_request = if attempt < self.retry_policy.max_attempts {
                request.try_clone()
            } else {
                None
            };

            let response = request
                .send()
                .await
                .with_context(|| format!("{method} {url} failed"))?;

            check_rate_limit(&response);

            let (response, rate_limit_body) = detect_rate_limit_body(response).await?;
            let status = response.status();

            // The attempt that failed with a 5xx may have deleted the resource.
            if method == "DELETE"
                && retried_after_server_error
                && status == reqwest::StatusCode::NOT_FOUND
            {
                tracing::debug!(
                    "{method} {url} returned HTTP 404 after a retry. The first attempt already deleted it"
                );
                return Ok(no_content_response());
            }

            let Some(plan) = response::retry_delay(
                status,
                response.headers(),
                rate_limit_body,
                attempt,
                &self.retry_policy.timing(),
                Utc::now(),
            ) else {
                if response::is_rate_limit_status(status)
                    && !response::needs_body_check(response.headers())
                {
                    tracing::warn!(
                        "{method} {url} returned HTTP {status} with an exhausted rate limit that is too long to wait for"
                    );
                }
                return Ok(response);
            };
            if plan.kind == response::RetryKind::Transient && !retry_server_errors {
                tracing::debug!("{method} {url} returned HTTP {status} and is not retried");
                return Ok(response);
            }
            let delay = plan.delay;

            let Some(next_request) = retry_request else {
                tracing::debug!(
                    "{method} {url} returned HTTP {} but the request cannot be retried safely",
                    status
                );
                return Ok(response);
            };

            match plan.kind {
                response::RetryKind::Transient => tracing::debug!(
                    "{method} {url} returned HTTP {status} and will be retried after {delay:?} ({}/{})",
                    attempt + 1,
                    self.retry_policy.max_attempts
                ),
                response::RetryKind::PrimaryRateLimit | response::RetryKind::SecondaryRateLimit => {
                    tracing::warn!(
                        "GitHub {} rate limit hit on {method} {url} (HTTP {status}). Waiting {delay:?} before retry {}/{}",
                        if plan.kind == response::RetryKind::PrimaryRateLimit {
                            "primary"
                        } else {
                            "secondary"
                        },
                        attempt + 1,
                        self.retry_policy.max_attempts
                    );
                }
            }

            if plan.kind == response::RetryKind::Transient {
                retried_after_server_error = true;
            }
            tokio::time::sleep(delay).await;
            request = next_request;
            attempt += 1;
        }
    }
}

fn no_content_response() -> reqwest::Response {
    let mut response = http::Response::new("");
    *response.status_mut() = reqwest::StatusCode::NO_CONTENT;
    reqwest::Response::from(response)
}

/// Read a 403 body unless `retry-after` already explains it, so a rate limit is
/// recognised by its message and never by `x-ratelimit-remaining` alone. The response is rebuilt
/// so callers can read the body again.
async fn detect_rate_limit_body(response: reqwest::Response) -> Result<(reqwest::Response, bool)> {
    if response.status() != reqwest::StatusCode::FORBIDDEN
        || !response::needs_body_check(response.headers())
    {
        return Ok((response, false));
    }

    let status = response.status();
    let version = response.version();
    let headers = response.headers().clone();
    let body = response
        .bytes()
        .await
        .context("Failed to read GitHub 403 response body")?;
    let secondary = response::mentions_rate_limit(&body);

    let mut rebuilt = http::Response::new(body);
    *rebuilt.status_mut() = status;
    *rebuilt.version_mut() = version;
    *rebuilt.headers_mut() = headers;
    Ok((reqwest::Response::from(rebuilt), secondary))
}

/// Only a plain query is safe to retry after a 5xx. A mutation may have been applied.
fn is_read_only_query(query: &str) -> bool {
    let query = query.trim_start();
    query.starts_with("query") || query.starts_with('{')
}

fn check_rate_limit(resp: &reqwest::Response) {
    let headers = resp.headers();
    if let Some(remaining) = low_rate_limit_remaining(headers) {
        let resource = headers
            .get("x-ratelimit-resource")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown");
        tracing::warn!("GitHub API rate limit low for {resource}: {remaining} remaining");
    }
}

/// Returns the remaining count when it is under 10% of the resource limit.
/// Limits differ per resource (search allows 30 per minute, core 5000 per hour),
/// so a fixed count would warn on every search run.
fn low_rate_limit_remaining(headers: &HeaderMap) -> Option<u32> {
    let number = |name: &str| headers.get(name)?.to_str().ok()?.trim().parse::<u32>().ok();
    let remaining = number("x-ratelimit-remaining")?;
    let limit = number("x-ratelimit-limit")?;
    (u64::from(remaining) * 10 < u64::from(limit)).then_some(remaining)
}

fn default_headers(mut authorization: HeaderValue) -> Result<HeaderMap> {
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static(metadata::REST_API_VERSION),
    );
    headers.insert(header::AUTHORIZATION, authorization);
    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_str(metadata::USER_AGENT).context("Invalid user agent header")?,
    );
    Ok(headers)
}

const SECONDARY_RATE_LIMIT_WAIT: Duration = Duration::from_secs(60);
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug)]
struct RetryPolicy {
    max_attempts: usize,
    backoff_schedule: [Duration; 3],
    secondary_wait: Duration,
    max_rate_limit_wait: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: metadata::MAX_RETRY_ATTEMPTS,
            backoff_schedule: metadata::RETRY_BACKOFF_SCHEDULE,
            secondary_wait: SECONDARY_RATE_LIMIT_WAIT,
            max_rate_limit_wait: MAX_RATE_LIMIT_WAIT,
        }
    }
}

impl RetryPolicy {
    fn immediate_for_tests() -> Self {
        Self {
            max_attempts: metadata::MAX_RETRY_ATTEMPTS,
            backoff_schedule: [Duration::ZERO; 3],
            secondary_wait: Duration::ZERO,
            max_rate_limit_wait: MAX_RATE_LIMIT_WAIT,
        }
    }

    fn timing(&self) -> response::RetryTiming<'_> {
        response::RetryTiming {
            backoff_schedule: &self.backoff_schedule,
            secondary_wait: self.secondary_wait,
            max_rate_limit_wait: self.max_rate_limit_wait,
        }
    }
}

fn validate_parallelism(parallelism: usize) -> Result<()> {
    if parallelism == 0 {
        anyhow::bail!("GitHub client parallelism must be at least 1");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderMap, HeaderValue};

    use super::{Client, default_headers, is_read_only_query, low_rate_limit_remaining};

    fn limit_headers(remaining: &'static str, limit: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", HeaderValue::from_static(remaining));
        headers.insert("x-ratelimit-limit", HeaderValue::from_static(limit));
        headers
    }

    #[test]
    fn authorization_header_is_marked_sensitive() {
        let headers = default_headers(HeaderValue::from_static("Bearer secret")).unwrap();
        let authorization = &headers[reqwest::header::AUTHORIZATION];
        assert!(authorization.is_sensitive());
        assert!(!format!("{authorization:?}").contains("secret"));
    }

    #[test]
    fn rate_limit_warning_scales_with_the_resource_limit() {
        assert_eq!(low_rate_limit_remaining(&limit_headers("29", "30")), None);
        assert_eq!(
            low_rate_limit_remaining(&limit_headers("50", "5000")),
            Some(50)
        );
        assert_eq!(low_rate_limit_remaining(&limit_headers("2", "30")), Some(2));
        assert_eq!(
            low_rate_limit_remaining(&limit_headers("4000", "5000")),
            None
        );
        assert_eq!(low_rate_limit_remaining(&HeaderMap::new()), None);
    }

    #[test]
    fn only_plain_queries_are_read_only() {
        assert!(is_read_only_query("  query Viewer { viewer { login } }"));
        assert!(is_read_only_query("{ viewer { login } }"));
        assert!(!is_read_only_query("mutation M { x }"));
        assert!(!is_read_only_query("# note\nmutation M { x }"));
    }

    fn assert_send<T: Send>(_: T) {}

    #[allow(dead_code)]
    fn request_futures_are_send<B: serde::Serialize + Sync, V: serde::Serialize + Sync>(
        client: &Client,
        body: &B,
        variables: &V,
    ) {
        assert_send(client.patch_json("/", body));
        assert_send(client.post_json("/", body));
        assert_send(client.put_json("/", body));
        assert_send(client.delete_json("/", body));
        assert_send(client.graphql::<serde_json::Value, V>("query { x }", variables));
    }

    #[test]
    fn client_new_rejects_zero_parallelism() {
        let Err(error) = Client::new("test-org", 0) else {
            panic!("parallelism=0 must be rejected");
        };

        assert!(
            error
                .to_string()
                .contains("GitHub client parallelism must be at least 1")
        );
    }
}
