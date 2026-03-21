use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;

use super::Client;

#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
    pub archived: bool,
    pub default_branch: String,
    #[serde(default)]
    pub description: Option<String>,
    pub visibility: String,
    #[serde(default)]
    pub language: Option<String>,
}

impl Client {
    /// List all repositories for the configured org, handling pagination.
    pub async fn list_repos(&self) -> Result<Vec<Repository>> {
        let mut all_repos = Vec::new();
        let mut page = 1u32;

        loop {
            let resp = self
                .get(&format!(
                    "/orgs/{}/repos?per_page=100&page={page}&type=all",
                    self.org
                ))
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Failed to list repos (HTTP {status}): {body}");
            }

            let repos: Vec<Repository> = resp
                .json()
                .await
                .context("Failed to parse repo list response")?;

            if repos.is_empty() {
                break;
            }

            all_repos.extend(repos);
            page += 1;
        }

        Ok(all_repos)
    }

    /// List repos filtered by system ID and exclude patterns.
    pub async fn list_repos_for_system(
        &self,
        system_id: &str,
        exclude_patterns: &[String],
    ) -> Result<Vec<Repository>> {
        let all = self.list_repos().await?;

        let exclude_regex = if exclude_patterns.is_empty() {
            None
        } else {
            let pattern = exclude_patterns.join("|");
            Some(Regex::new(&pattern).context("Invalid exclude pattern regex")?)
        };

        let filtered: Vec<Repository> = all
            .into_iter()
            .filter(|r| !r.archived)
            .filter(|r| r.name.starts_with(system_id))
            .filter(|r| {
                if let Some(ref re) = exclude_regex {
                    let suffix = r.name.strip_prefix(system_id).unwrap_or(&r.name);
                    let suffix = suffix.strip_prefix('-').unwrap_or(suffix);
                    !re.is_match(suffix)
                } else {
                    true
                }
            })
            .collect();

        Ok(filtered)
    }

    /// Get a single repository.
    pub async fn get_repo(&self, repo_name: &str) -> Result<Repository> {
        let resp = self
            .get(&format!("/repos/{}/{repo_name}", self.org))
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get repo {repo_name} (HTTP {status}): {body}");
        }

        resp.json()
            .await
            .context("Failed to parse repo response")
    }
}
