use anyhow::{Context, Result};
use serde::Deserialize;

use super::Client;

#[derive(Debug, Deserialize)]
pub struct FileContent {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(default)]
    pub content: Option<String>,
    pub encoding: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
}

impl Client {
    /// Get a file's content from a repo. Returns None if the file doesn't exist.
    pub async fn get_file(
        &self,
        repo: &str,
        path: &str,
        branch: Option<&str>,
    ) -> Result<Option<FileContent>> {
        let mut url = format!("/repos/{}/{repo}/contents/{path}", self.org);
        if let Some(branch) = branch {
            url.push_str(&format!("?ref={branch}"));
        }

        let resp = self.get(&url).await?;

        match resp.status().as_u16() {
            200 => {
                let content = resp
                    .json()
                    .await
                    .context("Failed to parse file content response")?;
                Ok(Some(content))
            }
            404 => Ok(None),
            status => {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Failed to get file {path} in {repo} (HTTP {status}): {body}");
            }
        }
    }

    /// Decode base64-encoded file content from the Contents API.
    pub fn decode_content(content: &FileContent) -> Result<String> {
        let raw = content.content.as_deref().unwrap_or("");
        let cleaned: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &cleaned)
            .context("Failed to decode base64 content")?;
        String::from_utf8(bytes).context("File content is not valid UTF-8")
    }

    /// List a directory's entries from the Contents API. Returns `None` if the path doesn't exist.
    pub async fn list_directory(
        &self,
        repo: &str,
        path: &str,
        branch: Option<&str>,
    ) -> Result<Option<Vec<DirectoryEntry>>> {
        let mut url = if path.is_empty() {
            format!("/repos/{}/{repo}/contents", self.org)
        } else {
            format!("/repos/{}/{repo}/contents/{path}", self.org)
        };
        if let Some(branch) = branch {
            url.push_str(&format!("?ref={branch}"));
        }

        let resp = self.get(&url).await?;

        match resp.status().as_u16() {
            200 => {
                let entries = resp
                    .json()
                    .await
                    .context("Failed to parse directory listing response")?;
                Ok(Some(entries))
            }
            404 => Ok(None),
            status => {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Failed to list directory {path} in {repo} (HTTP {status}): {body}");
            }
        }
    }
}
