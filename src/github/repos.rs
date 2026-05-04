use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::Client;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Pre-fetched security_and_analysis data from the repo listing response.
    #[serde(default)]
    pub security_and_analysis: Option<serde_json::Value>,
    /// Repository topics (tags) from GitHub.
    #[serde(default)]
    pub topics: Vec<String>,
}

/// Response wrapper for the GitHub search repositories API.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    items: Vec<Repository>,
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

    /// Search for repos matching a name query within the configured org.
    /// Uses the GitHub search API which is much faster than listing all repos
    /// and filtering client-side.
    async fn search_repos_by_name(&self, name_query: &str) -> Result<Vec<Repository>> {
        let mut all_repos = Vec::new();
        let mut page = 1u32;
        // Build query: org:<org>+<name_query>+in:name
        // The `+` in the query string acts as a space/AND for GitHub search.
        let query = format!("org:{}+{}+in:name", self.org, name_query);

        loop {
            let resp = self
                .get(&format!(
                    "/search/repositories?q={query}&per_page=100&page={page}"
                ))
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Failed to search repos (HTTP {status}): {body}");
            }

            let search_result: SearchResponse = resp
                .json()
                .await
                .context("Failed to parse search response")?;

            if search_result.items.is_empty() {
                break;
            }

            all_repos.extend(search_result.items);
            page += 1;
        }

        Ok(all_repos)
    }

    /// List repos filtered by system ID prefix and/or explicit repo names,
    /// with exclude patterns applied to the combined result.
    ///
    /// Uses the GitHub search API for prefix matching, which avoids fetching
    /// every repo in the org just to find the few that match.
    pub async fn list_repos_for_system(
        &self,
        system_id: &str,
        exclude_patterns: &[String],
        explicit_repos: &[String],
    ) -> Result<Vec<Repository>> {
        let search_results = self.search_repos_by_name(system_id).await?;

        let exclude_regex = if exclude_patterns.is_empty() {
            None
        } else {
            let pattern = exclude_patterns.join("|");
            Some(Regex::new(&pattern).context("Invalid exclude pattern regex")?)
        };

        // The search API matches system_id anywhere in the name, so we still
        // need to verify the prefix to avoid false positives.
        // Require exact match or `{system_id}-` prefix to avoid e.g. "be" matching "backend".
        let prefix_with_sep = format!("{system_id}-");
        let mut matched: Vec<Repository> = search_results
            .into_iter()
            .filter(|r| !r.archived)
            .filter(|r| r.name == system_id || r.name.starts_with(&prefix_with_sep))
            .filter(|r| {
                if let Some(ref re) = exclude_regex {
                    let suffix = r
                        .name
                        .strip_prefix(system_id)
                        .and_then(|s| s.strip_prefix('-'))
                        .unwrap_or(&r.name);
                    !re.is_match(suffix)
                } else {
                    true
                }
            })
            .collect();

        // Add explicit repos (fetch individually, skip if already matched by search)
        for repo_name in explicit_repos {
            if matched.iter().any(|r| r.name == *repo_name) {
                continue;
            }
            match self.get_repo(repo_name).await {
                Ok(repo) if !repo.archived => {
                    matched.push(repo);
                }
                Ok(_) => {} // archived, skip
                Err(e) => {
                    tracing::warn!("Failed to fetch explicit repo {repo_name}: {e}");
                }
            }
        }

        Ok(matched)
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

        resp.json().await.context("Failed to parse repo response")
    }
}
