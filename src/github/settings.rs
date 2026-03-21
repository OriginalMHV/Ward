use anyhow::Result;
use serde::Deserialize;

use super::Client;

#[derive(Debug, Deserialize)]
pub struct RepoSettings {
    pub has_issues: bool,
    pub has_projects: bool,
    pub has_wiki: bool,
    pub allow_squash_merge: bool,
    pub allow_merge_commit: bool,
    pub allow_rebase_merge: bool,
    pub delete_branch_on_merge: bool,
}

impl Client {
    /// Get repository settings.
    pub async fn get_settings(&self, repo: &str) -> Result<RepoSettings> {
        let resp = self.get(&format!("/repos/{}/{repo}", self.org)).await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get settings for {repo} (HTTP {status}): {body}");
        }

        Ok(resp.json().await?)
    }

    /// Update repository settings.
    pub async fn update_settings(&self, repo: &str, settings: &serde_json::Value) -> Result<()> {
        let resp = self
            .patch_json(&format!("/repos/{}/{repo}", self.org), settings)
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to update settings for {repo} (HTTP {status}): {body}");
        }

        Ok(())
    }
}
