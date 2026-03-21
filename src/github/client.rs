use std::sync::Arc;

use anyhow::{Context, Result};
use reqwest::header::{self, HeaderMap, HeaderValue};
use tokio::sync::Semaphore;

use crate::config::auth;

/// GitHub API client with rate limiting and concurrency control.
pub struct Client {
    pub http: reqwest::Client,
    pub org: String,
    pub semaphore: Arc<Semaphore>,
    pub base_url: String,
}

impl Client {
    pub async fn new(org: &str, parallelism: usize) -> Result<Self> {
        let token = auth::resolve_token()?;

        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("Invalid token characters")?,
        );
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("ward-cli/0.1.0"),
        );

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            http,
            org: org.to_owned(),
            semaphore: Arc::new(Semaphore::new(parallelism)),
            base_url: "https://api.github.com".to_owned(),
        })
    }

    /// Make a GET request to the GitHub API.
    pub async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let _permit = self.semaphore.acquire().await?;
        let url = format!("{}{}", self.base_url, path);

        tracing::debug!("GET {url}");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;

        check_rate_limit(&resp);
        Ok(resp)
    }

    /// Make a PUT request to the GitHub API.
    pub async fn put(&self, path: &str) -> Result<reqwest::Response> {
        let _permit = self.semaphore.acquire().await?;
        let url = format!("{}{}", self.base_url, path);

        tracing::debug!("PUT {url}");

        let resp = self
            .http
            .put(&url)
            .header(header::CONTENT_LENGTH, 0)
            .send()
            .await
            .with_context(|| format!("PUT {url} failed"))?;

        check_rate_limit(&resp);
        Ok(resp)
    }

    /// Make a PATCH request with a JSON body.
    pub async fn patch_json<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let _permit = self.semaphore.acquire().await?;
        let url = format!("{}{}", self.base_url, path);

        tracing::debug!("PATCH {url}");

        let resp = self
            .http
            .patch(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("PATCH {url} failed"))?;

        check_rate_limit(&resp);
        Ok(resp)
    }

    /// Make a POST request with a JSON body.
    pub async fn post_json<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let _permit = self.semaphore.acquire().await?;
        let url = format!("{}{}", self.base_url, path);

        tracing::debug!("POST {url}");

        let resp = self
            .http
            .post(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url} failed"))?;

        check_rate_limit(&resp);
        Ok(resp)
    }

    /// Make a DELETE request.
    pub async fn delete(&self, path: &str) -> Result<reqwest::Response> {
        let _permit = self.semaphore.acquire().await?;
        let url = format!("{}{}", self.base_url, path);

        tracing::debug!("DELETE {url}");

        let resp = self
            .http
            .delete(&url)
            .send()
            .await
            .with_context(|| format!("DELETE {url} failed"))?;

        check_rate_limit(&resp);
        Ok(resp)
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
