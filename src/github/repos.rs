use crate::github::error::ReadContext;
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::Client;
use super::pagination;
use super::response::{self, ClassifiedResponse};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<RepositoryOwner>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryOwner {
    pub login: String,
    #[serde(rename = "type")]
    pub kind: String,
}

impl Repository {
    /// True unless GitHub reports a non-organization owner, so unknown owners keep org behaviour.
    pub fn is_organization_owned(&self) -> bool {
        self.owner
            .as_ref()
            .is_none_or(|owner| owner.kind == "Organization")
    }
}

/// Whether the configured owner is an organization or a personal account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKind {
    Organization,
    User,
}

#[derive(Deserialize)]
struct OwnerProfile {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct AuthenticatedUser {
    login: String,
}

/// Response wrapper for the GitHub search repositories API.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    items: Vec<Repository>,
}

impl Client {
    /// Detect whether the configured owner is an organization or a user.
    ///
    /// The result is cached for the run. An owner that GitHub cannot find is treated as an
    /// organization, so the later reads report the missing owner with their own errors.
    pub async fn owner_kind(&self) -> Result<OwnerKind> {
        self.cached_org(|lookups| &lookups.owner_kind, self.fetch_owner_kind())
            .await
    }

    async fn fetch_owner_kind(&self) -> Result<OwnerKind> {
        let path = format!("/users/{}", self.org);
        let classified =
            response::classify_json::<OwnerProfile>(self.get(&path).await?, "GET", &path)
                .await
                .read_ctx("owner lookup")?;
        match classified {
            ClassifiedResponse::Success(profile) if profile.kind == "User" => Ok(OwnerKind::User),
            ClassifiedResponse::Success(_)
            | ClassifiedResponse::NoContent
            | ClassifiedResponse::NotFound(_) => Ok(OwnerKind::Organization),
            ClassifiedResponse::Forbidden(error)
            | ClassifiedResponse::Unprocessable(error)
            | ClassifiedResponse::Other(error) => Err(error).read_ctx("owner lookup"),
        }
    }

    /// The login of the token owner, or `None` when the token has no user (for example a GitHub App token).
    async fn authenticated_login(&self) -> Result<Option<String>> {
        self.cached_org(
            |lookups| &lookups.authenticated_login,
            self.fetch_authenticated_login(),
        )
        .await
    }

    async fn fetch_authenticated_login(&self) -> Result<Option<String>> {
        let path = "/user";
        let classified =
            response::classify_json::<AuthenticatedUser>(self.get(path).await?, "GET", path)
                .await
                .read_ctx("authenticated user lookup")?;
        match classified {
            ClassifiedResponse::Success(user) => Ok(Some(user.login)),
            _ => Ok(None),
        }
    }

    /// List all repositories for the configured owner, handling pagination.
    ///
    /// A user owner is listed through `GET /user/repos?affiliation=owner` when it is the
    /// token owner, so private repositories are included. Other users list public repositories only.
    pub async fn list_repos(&self) -> Result<Vec<Repository>> {
        if self.owner_kind().await? == OwnerKind::User {
            let own_account = self
                .authenticated_login()
                .await?
                .is_some_and(|login| login.eq_ignore_ascii_case(&self.org));
            return pagination::collect_paginated(self, |page| {
                if own_account {
                    format!(
                        "/user/repos?affiliation=owner&per_page={}&page={}",
                        page.per_page, page.number
                    )
                } else {
                    format!(
                        "/users/{}/repos?type=owner&per_page={}&page={}",
                        self.org, page.per_page, page.number
                    )
                }
            })
            .await
            .read_ctx("repo list");
        }
        pagination::collect_paginated(self, |page| {
            format!(
                "/orgs/{}/repos?per_page={}&page={}&type=all",
                self.org, page.per_page, page.number
            )
        })
        .await
        .read_ctx("repo list")
    }

    /// Search for repos matching a name query within the configured org.
    /// Uses the GitHub search API which is much faster than listing all repos
    /// and filtering client-side.
    async fn search_repos_by_name(&self, name_query: &str) -> Result<Vec<Repository>> {
        // Build query: <org|user>:<owner>+<name_query>+in:name
        // The `+` in the query string acts as a space/AND for GitHub search.
        let qualifier = match self.owner_kind().await? {
            OwnerKind::Organization => "org",
            OwnerKind::User => "user",
        };
        let query = format!("{qualifier}:{}+{}+in:name", self.org, name_query);
        let mut page = 1u32;
        let mut all_repos = Vec::new();

        loop {
            let path = format!(
                "/search/repositories?q={query}&per_page={}&page={page}",
                pagination::PAGE_SIZE
            );
            let search_result: SearchResponse =
                response::expect_json(self.get(&path).await?, "GET", &path)
                    .await
                    .read_ctx("search")?;
            let item_count = search_result.items.len();
            all_repos.extend(search_result.items);

            if item_count < pagination::PAGE_SIZE as usize {
                break;
            }

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
        match_prefix: bool,
        exclude_patterns: &[String],
        explicit_repos: &[String],
    ) -> Result<Vec<Repository>> {
        let search_results = if match_prefix {
            self.search_repos_by_name(system_id).await?
        } else {
            Vec::new()
        };

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

        // Add explicit repos (fetch concurrently, skip if already matched by search)
        let pending: Vec<&String> = explicit_repos
            .iter()
            .filter(|repo_name| !matched.iter().any(|r| r.name == **repo_name))
            .collect();
        let fetched =
            futures_util::future::try_join_all(pending.into_iter().map(|repo_name| async move {
                self.get_repo(repo_name).await.with_context(|| {
                    format!("Could not load the repository {repo_name} that the manifest names")
                })
            }))
            .await?;
        for repo in fetched {
            if !repo.archived && !matched.iter().any(|r| r.name == repo.name) {
                matched.push(repo);
            }
        }

        Ok(matched)
    }

    /// Get a single repository as raw JSON, for callers that deserialize it
    /// into several views and want one request instead of one per view.
    pub async fn get_repo_value(&self, repo_name: &str) -> Result<serde_json::Value> {
        let path = format!("/repos/{}/{repo_name}", self.org);
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .read_ctx("repo")
    }

    /// Get a single repository.
    pub async fn get_repo(&self, repo_name: &str) -> Result<Repository> {
        let path = format!("/repos/{}/{repo_name}", self.org);
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .read_ctx("repo")
    }
}
