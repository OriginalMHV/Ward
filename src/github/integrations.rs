//! Repository webhook, deploy key, Pages, autolink, label, and custom-property APIs.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::config::manifest::{AutolinkConfigV2, PagesConfigV2, WebhookConfigV2};

use super::Client;
use super::actions::{ReadOutcome, classify_read};
use super::pagination;
use super::response;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RepositoryWebhook {
    pub id: u64,
    pub url: String,
    pub active: bool,
    pub events: Vec<String>,
    pub content_type: Option<String>,
    pub insecure_ssl: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RepositoryDeployKey {
    pub id: u64,
    pub title: String,
    pub read_only: bool,
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RepositoryPagesSite {
    pub build_type: Option<String>,
    pub source_branch: Option<String>,
    pub source_path: Option<String>,
    pub cname: Option<String>,
    pub https_enforced: Option<bool>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RepositoryAutolink {
    pub id: u64,
    pub key_prefix: String,
    pub url_template: String,
    #[serde(default)]
    pub is_alphanumeric: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct WebhookApiResponse {
    id: u64,
    active: bool,
    #[serde(default)]
    events: Vec<String>,
    config: WebhookConfigApiResponse,
}

#[derive(Debug, Deserialize)]
struct WebhookConfigApiResponse {
    url: String,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    insecure_ssl: Option<Value>,
}

impl From<WebhookApiResponse> for RepositoryWebhook {
    fn from(value: WebhookApiResponse) -> Self {
        Self {
            id: value.id,
            url: value.config.url,
            active: value.active,
            events: value.events,
            content_type: value.config.content_type,
            insecure_ssl: value
                .config
                .insecure_ssl
                .as_ref()
                .and_then(parse_insecure_ssl),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeployKeyApiResponse {
    id: u64,
    title: String,
    read_only: bool,
    #[serde(default)]
    fingerprint: Option<String>,
}

impl From<DeployKeyApiResponse> for RepositoryDeployKey {
    fn from(value: DeployKeyApiResponse) -> Self {
        Self {
            id: value.id,
            title: value.title,
            read_only: value.read_only,
            fingerprint: value.fingerprint,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PagesApiResponse {
    #[serde(default)]
    build_type: Option<String>,
    #[serde(default)]
    source: Option<PagesSourceApiResponse>,
    #[serde(default)]
    cname: Option<String>,
    #[serde(default)]
    https_enforced: Option<bool>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PagesSourceApiResponse {
    branch: String,
    path: String,
}

impl From<PagesApiResponse> for RepositoryPagesSite {
    fn from(value: PagesApiResponse) -> Self {
        Self {
            build_type: value.build_type,
            source_branch: value.source.as_ref().map(|source| source.branch.clone()),
            source_path: value.source.map(|source| source.path),
            cname: value.cname,
            https_enforced: value.https_enforced,
            status: value.status,
        }
    }
}

impl Client {
    pub async fn list_repo_webhooks(&self, repo: &str) -> Result<Vec<RepositoryWebhook>> {
        self.list_repo_webhooks_checked(repo)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("repository webhooks unavailable"))
    }

    pub async fn list_repo_webhooks_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<RepositoryWebhook>>> {
        collect_paginated_checked::<WebhookApiResponse, _>(self, |page| {
            format!(
                "/repos/{}/{repo}/hooks?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse repo webhooks response")
        .map(|outcome| match outcome {
            ReadOutcome::Available(items) => {
                ReadOutcome::Available(items.into_iter().map(RepositoryWebhook::from).collect())
            }
            ReadOutcome::NotApplicable(reason) => ReadOutcome::NotApplicable(reason),
            ReadOutcome::PermissionDenied(reason) => ReadOutcome::PermissionDenied(reason),
            ReadOutcome::Unavailable(reason) => ReadOutcome::Unavailable(reason),
        })
    }

    pub async fn create_repo_webhook(
        &self,
        repo: &str,
        webhook: &WebhookConfigV2,
        resolved_url: &str,
        secret: Option<&str>,
    ) -> Result<RepositoryWebhook> {
        let body = webhook_create_request_body(webhook, resolved_url, secret);
        let path = format!("/repos/{}/{repo}/hooks", self.org);
        let response: WebhookApiResponse =
            response::expect_json(self.post_json(&path, &body).await?, "POST", &path).await?;
        Ok(response.into())
    }

    pub async fn update_repo_webhook(
        &self,
        repo: &str,
        hook_id: u64,
        config_patch: WebhookConfigPatch<'_>,
        metadata_patch: WebhookMetadataPatch<'_>,
    ) -> Result<()> {
        if config_patch.has_changes() {
            let body = webhook_config_patch_body(config_patch);
            let path = format!("/repos/{}/{repo}/hooks/{hook_id}/config", self.org);
            response::expect_json::<Value>(self.patch_json(&path, &body).await?, "PATCH", &path)
                .await
                .map(|_| ())?;
        }
        if metadata_patch.has_changes() {
            let body = webhook_metadata_patch_body(metadata_patch);
            let path = format!("/repos/{}/{repo}/hooks/{hook_id}", self.org);
            response::expect_json::<Value>(self.patch_json(&path, &body).await?, "PATCH", &path)
                .await
                .map(|_| ())?;
        }
        Ok(())
    }

    pub async fn delete_repo_webhook(&self, repo: &str, hook_id: u64) -> Result<()> {
        let path = format!("/repos/{}/{repo}/hooks/{hook_id}", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn list_repo_deploy_keys(&self, repo: &str) -> Result<Vec<RepositoryDeployKey>> {
        self.list_repo_deploy_keys_checked(repo)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("repository deploy keys unavailable"))
    }

    pub async fn list_repo_deploy_keys_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<RepositoryDeployKey>>> {
        collect_paginated_checked::<DeployKeyApiResponse, _>(self, |page| {
            format!(
                "/repos/{}/{repo}/keys?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse repo deploy keys response")
        .map(|outcome| match outcome {
            ReadOutcome::Available(items) => {
                ReadOutcome::Available(items.into_iter().map(RepositoryDeployKey::from).collect())
            }
            ReadOutcome::NotApplicable(reason) => ReadOutcome::NotApplicable(reason),
            ReadOutcome::PermissionDenied(reason) => ReadOutcome::PermissionDenied(reason),
            ReadOutcome::Unavailable(reason) => ReadOutcome::Unavailable(reason),
        })
    }

    pub async fn create_repo_deploy_key(
        &self,
        repo: &str,
        title: &str,
        public_key: &str,
        read_only: bool,
    ) -> Result<()> {
        let path = format!("/repos/{}/{repo}/keys", self.org);
        let body = serde_json::json!({
            "title": title,
            "key": public_key,
            "read_only": read_only,
        });
        response::expect_json::<Value>(self.post_json(&path, &body).await?, "POST", &path)
            .await
            .map(|_| ())
    }

    pub async fn delete_repo_deploy_key(&self, repo: &str, key_id: u64) -> Result<()> {
        let path = format!("/repos/{}/{repo}/keys/{key_id}", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn get_repo_pages(&self, repo: &str) -> Result<Option<RepositoryPagesSite>> {
        self.get_repo_pages_checked(repo)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("repository pages unavailable"))
    }

    pub async fn get_repo_pages_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Option<RepositoryPagesSite>>> {
        let path = format!("/repos/{}/{repo}/pages", self.org);
        match response::classify_json(self.get(&path).await?, "GET", &path).await? {
            response::ClassifiedResponse::Success(value) => {
                let value: PagesApiResponse = value;
                Ok(ReadOutcome::Available(Some(value.into())))
            }
            response::ClassifiedResponse::NotFound(error) => {
                Ok(ReadOutcome::NotApplicable(error.to_string()))
            }
            response::ClassifiedResponse::Forbidden(error) => {
                Ok(ReadOutcome::PermissionDenied(error.to_string()))
            }
            response::ClassifiedResponse::Unprocessable(error) => {
                Ok(ReadOutcome::NotApplicable(error.to_string()))
            }
            response::ClassifiedResponse::Other(error) => {
                Ok(ReadOutcome::Unavailable(error.to_string()))
            }
            response::ClassifiedResponse::NoContent => Ok(ReadOutcome::Available(None)),
        }
    }

    pub async fn create_repo_pages(&self, repo: &str, pages: &PagesConfigV2) -> Result<()> {
        let body = pages_request_body(pages);
        let path = format!("/repos/{}/{repo}/pages", self.org);
        response::expect_json::<Value>(self.post_json(&path, &body).await?, "POST", &path)
            .await
            .map(|_| ())
    }

    pub async fn update_repo_pages(&self, repo: &str, pages: &PagesConfigV2) -> Result<()> {
        let body = pages_request_body(pages);
        let path = format!("/repos/{}/{repo}/pages", self.org);
        response::expect_json::<Value>(self.put_json(&path, &body).await?, "PUT", &path)
            .await
            .map(|_| ())
    }

    pub async fn delete_repo_pages(&self, repo: &str) -> Result<()> {
        let path = format!("/repos/{}/{repo}/pages", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn list_repo_autolinks(&self, repo: &str) -> Result<Vec<RepositoryAutolink>> {
        self.list_repo_autolinks_checked(repo)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("repository autolinks unavailable"))
    }

    pub async fn list_repo_autolinks_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<RepositoryAutolink>>> {
        collect_paginated_checked(self, |page| {
            format!(
                "/repos/{}/{repo}/autolinks?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse repo autolinks response")
    }

    pub async fn create_repo_autolink(
        &self,
        repo: &str,
        autolink: &AutolinkConfigV2,
    ) -> Result<()> {
        let path = format!("/repos/{}/{repo}/autolinks", self.org);
        let mut body = serde_json::Map::new();
        body.insert(
            "key_prefix".to_owned(),
            Value::String(autolink.key_prefix.clone()),
        );
        body.insert(
            "url_template".to_owned(),
            Value::String(autolink.url_template.clone()),
        );
        if let Some(is_alphanumeric) = autolink.is_alphanumeric {
            body.insert("is_alphanumeric".to_owned(), Value::Bool(is_alphanumeric));
        }
        response::expect_json::<Value>(
            self.post_json(&path, &Value::Object(body)).await?,
            "POST",
            &path,
        )
        .await
        .map(|_| ())
    }

    pub async fn delete_repo_autolink(&self, repo: &str, autolink_id: u64) -> Result<()> {
        let path = format!("/repos/{}/{repo}/autolinks/{autolink_id}", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WebhookConfigPatch<'a> {
    pub url: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub insecure_ssl: Option<bool>,
    pub secret: Option<&'a str>,
}

impl WebhookConfigPatch<'_> {
    fn has_changes(self) -> bool {
        self.url.is_some()
            || self.content_type.is_some()
            || self.insecure_ssl.is_some()
            || self.secret.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WebhookMetadataPatch<'a> {
    pub active: Option<bool>,
    pub events: Option<&'a [String]>,
}

impl WebhookMetadataPatch<'_> {
    fn has_changes(self) -> bool {
        self.active.is_some() || self.events.is_some()
    }
}

fn parse_insecure_ssl(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(boolean) => Some(*boolean),
        Value::String(text) => match text.as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        Value::Number(number) => number.as_u64().map(|value| value != 0),
        _ => None,
    }
}

fn webhook_create_request_body(
    webhook: &WebhookConfigV2,
    resolved_url: &str,
    secret: Option<&str>,
) -> Value {
    let mut config = Map::new();
    config.insert("url".to_owned(), Value::String(resolved_url.to_owned()));

    if let Some(content_type) = &webhook.content_type {
        config.insert(
            "content_type".to_owned(),
            Value::String(content_type.clone()),
        );
    }

    if let Some(insecure_ssl) = webhook.insecure_ssl {
        config.insert(
            "insecure_ssl".to_owned(),
            Value::String(if insecure_ssl { "1" } else { "0" }.to_owned()),
        );
    }

    if let Some(secret) = secret {
        config.insert("secret".to_owned(), Value::String(secret.to_owned()));
    }

    let mut payload = Map::new();
    payload.insert("config".to_owned(), Value::Object(config));

    if let Some(active) = webhook.active {
        payload.insert("active".to_owned(), Value::Bool(active));
    }

    if !webhook.events.is_empty() {
        payload.insert(
            "events".to_owned(),
            Value::Array(webhook.events.iter().cloned().map(Value::String).collect()),
        );
    }

    Value::Object(payload)
}

fn webhook_config_patch_body(patch: WebhookConfigPatch<'_>) -> Value {
    let mut payload = Map::new();
    if let Some(url) = patch.url {
        payload.insert("url".to_owned(), Value::String(url.to_owned()));
    }
    if let Some(content_type) = patch.content_type {
        payload.insert(
            "content_type".to_owned(),
            Value::String(content_type.to_owned()),
        );
    }
    if let Some(insecure_ssl) = patch.insecure_ssl {
        payload.insert(
            "insecure_ssl".to_owned(),
            Value::String(if insecure_ssl { "1" } else { "0" }.to_owned()),
        );
    }
    if let Some(secret) = patch.secret {
        payload.insert("secret".to_owned(), Value::String(secret.to_owned()));
    }
    Value::Object(payload)
}

fn webhook_metadata_patch_body(patch: WebhookMetadataPatch<'_>) -> Value {
    let mut payload = Map::new();
    if let Some(active) = patch.active {
        payload.insert("active".to_owned(), Value::Bool(active));
    }
    if let Some(events) = patch.events {
        payload.insert(
            "events".to_owned(),
            Value::Array(events.iter().cloned().map(Value::String).collect()),
        );
    }
    Value::Object(payload)
}

fn pages_request_body(pages: &PagesConfigV2) -> Value {
    let mut payload = Map::new();

    if let Some(build_type) = &pages.build_type {
        payload.insert("build_type".to_owned(), Value::String(build_type.clone()));
    }

    if let Some(cname) = &pages.cname {
        payload.insert("cname".to_owned(), Value::String(cname.clone()));
    }

    if let Some(https_enforced) = pages.https_enforced {
        payload.insert("https_enforced".to_owned(), Value::Bool(https_enforced));
    }

    let use_source = !matches!(pages.build_type.as_deref(), Some("workflow"));
    if use_source && let (Some(branch), Some(path)) = (&pages.source_branch, &pages.source_path) {
        let mut source = Map::new();
        source.insert("branch".to_owned(), Value::String(branch.clone()));
        source.insert("path".to_owned(), Value::String(path.clone()));
        payload.insert("source".to_owned(), Value::Object(source));
    }

    Value::Object(payload)
}

async fn collect_paginated_checked<T, F>(
    client: &Client,
    mut build_path: F,
) -> Result<ReadOutcome<Vec<T>>>
where
    T: for<'de> Deserialize<'de>,
    F: FnMut(pagination::Page) -> String,
{
    let mut page = pagination::Page::default();
    let mut items = Vec::new();

    loop {
        let path = build_path(page);
        let page_items: Vec<T> = match classify_read(client.get(&path).await?, "GET", &path, false)
            .await?
        {
            ReadOutcome::Available(values) => values,
            ReadOutcome::NotApplicable(reason) => return Ok(ReadOutcome::NotApplicable(reason)),
            ReadOutcome::PermissionDenied(reason) => {
                return Ok(ReadOutcome::PermissionDenied(reason));
            }
            ReadOutcome::Unavailable(reason) => return Ok(ReadOutcome::Unavailable(reason)),
        };
        let count = page_items.len();
        items.extend(page_items);
        if count < page.per_page as usize {
            break;
        }
        page = pagination::Page {
            number: page.number + 1,
            ..page
        };
    }

    Ok(ReadOutcome::Available(items))
}
