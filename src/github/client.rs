use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;

use crate::config::auth;

use super::metadata;
use super::response;

/// GitHub API client with rate limiting and concurrency control.
#[derive(Clone)]
pub struct Client {
    pub(crate) http: reqwest::Client,
    pub(crate) org: String,
    pub(crate) semaphore: Arc<Semaphore>,
    pub(crate) base_url: String,
    retry_policy: RetryPolicy,
}

impl Client {
    /// The GitHub organization this client targets.
    pub fn org(&self) -> &str {
        &self.org
    }

    pub async fn new(org: &str, parallelism: usize) -> Result<Self> {
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
    pub async fn patch_json<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("PATCH", &url, self.http.patch(&url).json(body))
            .await
    }

    /// Make a POST request with a JSON body.
    pub async fn post_json<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.send("POST", &url, self.http.post(&url).json(body))
            .await
    }

    /// Make a PUT request with a JSON body.
    pub async fn put_json<T: serde::Serialize>(
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

    pub async fn graphql<T, V>(&self, query: &str, variables: &V) -> Result<T>
    where
        T: DeserializeOwned,
        V: Serialize,
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
        let response = self
            .post_json(path, &GraphqlRequest { query, variables })
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
        }
    }

    async fn send(
        &self,
        method: &str,
        url: &str,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        let _permit = self.semaphore.acquire().await?;
        let mut request = request;
        let mut attempt = 1usize;

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

            let Some(delay) = response::retry_delay(
                response.status(),
                response.headers(),
                attempt,
                &self.retry_policy.backoff_schedule,
                Utc::now(),
            ) else {
                return Ok(response);
            };

            let Some(next_request) = retry_request else {
                tracing::debug!(
                    "{method} {url} returned HTTP {} but the request cannot be retried safely",
                    response.status()
                );
                return Ok(response);
            };

            tracing::debug!(
                "{method} {url} returned HTTP {} and will be retried after {:?} ({}/{})",
                response.status(),
                delay,
                attempt + 1,
                self.retry_policy.max_attempts
            );

            tokio::time::sleep(delay).await;
            request = next_request;
            attempt += 1;
        }
    }
}

fn check_rate_limit(resp: &reqwest::Response) {
    if let Some(remaining) = resp.headers().get("x-ratelimit-remaining")
        && let Ok(remaining) = remaining.to_str().unwrap_or("?").parse::<u32>()
        && remaining < 100
    {
        tracing::warn!("GitHub API rate limit low: {remaining} remaining");
    }
}

fn default_headers(authorization: HeaderValue) -> Result<HeaderMap> {
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

#[derive(Clone, Copy, Debug)]
struct RetryPolicy {
    max_attempts: usize,
    backoff_schedule: [Duration; 3],
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: metadata::MAX_RETRY_ATTEMPTS,
            backoff_schedule: metadata::RETRY_BACKOFF_SCHEDULE,
        }
    }
}

impl RetryPolicy {
    fn immediate_for_tests() -> Self {
        Self {
            max_attempts: metadata::MAX_RETRY_ATTEMPTS,
            backoff_schedule: [Duration::ZERO; 3],
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
    use super::Client;

    #[tokio::test]
    async fn client_new_rejects_zero_parallelism() {
        let error = match Client::new("test-org", 0).await {
            Ok(_) => panic!("parallelism=0 must be rejected"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("GitHub client parallelism must be at least 1")
        );
    }
}
