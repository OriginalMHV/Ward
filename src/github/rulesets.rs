use crate::github::error::ReadContext;
use anyhow::{Context, Result};
use serde::Deserialize;

use super::Client;
use super::actions::{ReadOutcome, classify_read};
use super::encoding::encode_unreserved;
use super::pagination;
use super::response;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Ruleset {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub source_type: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub enforcement: String,
    #[serde(default)]
    pub conditions: Option<serde_json::Value>,
    #[serde(default)]
    pub rules: Vec<RulesetRule>,
    #[serde(default)]
    pub bypass_actors: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RulesetDetail {
    pub id: u64,
    pub name: String,
    pub enforcement: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub rules: Vec<RulesetRule>,
    #[serde(default)]
    pub conditions: Option<serde_json::Value>,
    #[serde(default)]
    pub bypass_actors: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RulesetRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RulesetRepositoryCollaborator {
    pub id: u64,
    pub login: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RulesetCustomRepositoryRole {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct InstalledApp {
    pub app_id: u64,
    #[serde(default)]
    pub app_slug: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GitHubUser {
    pub id: u64,
    pub login: String,
}

#[derive(Debug, Deserialize)]
struct InstallationsResponse {
    #[serde(default)]
    total_count: Option<usize>,
    #[serde(default)]
    installations: Vec<InstalledApp>,
}

#[derive(Debug, Deserialize)]
struct BranchRule {
    #[serde(rename = "type")]
    rule_type: String,
}

impl Client {
    /// The rule types that apply to `branch`, from every ruleset that targets it.
    /// One request covers repository and inherited organization rulesets.
    pub async fn list_branch_rule_types(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<ReadOutcome<Vec<String>>> {
        let branch = encode_unreserved(branch);
        let path = format!(
            "/repos/{}/{repo}/rules/branches/{branch}?per_page=100",
            self.org
        );
        let outcome = classify_read::<Vec<BranchRule>>(self.get(&path).await?, "GET", &path, true)
            .await
            .read_ctx("branch rules")?;
        Ok(match outcome {
            ReadOutcome::Available(rules) => {
                ReadOutcome::Available(rules.into_iter().map(|rule| rule.rule_type).collect())
            }
            ReadOutcome::NotApplicable(reason) => ReadOutcome::NotApplicable(reason),
            ReadOutcome::PermissionDenied(reason) => ReadOutcome::PermissionDenied(reason),
            ReadOutcome::Unavailable(reason) => ReadOutcome::Unavailable(reason),
        })
    }

    pub async fn list_rulesets(&self, repo: &str) -> Result<Vec<Ruleset>> {
        self.list_rulesets_scoped(repo, true).await
    }

    async fn list_rulesets_scoped(
        &self,
        repo: &str,
        includes_parents: bool,
    ) -> Result<Vec<Ruleset>> {
        pagination::collect_paginated(self, |page| {
            format!(
                "/repos/{}/{repo}/rulesets?per_page={}&page={}&includes_parents={includes_parents}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to read rulesets response")
    }

    pub async fn get_ruleset(&self, repo: &str, ruleset_id: u64) -> Result<RulesetDetail> {
        let path = format!("/repos/{}/{repo}/rulesets/{ruleset_id}", self.org);
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .read_ctx("ruleset detail")
    }

    pub async fn create_ruleset(
        &self,
        repo: &str,
        ruleset: &serde_json::Value,
    ) -> Result<RulesetDetail> {
        let path = format!("/repos/{}/{repo}/rulesets", self.org);
        response::expect_json(self.post_json(&path, ruleset).await?, "POST", &path)
            .await
            .read_ctx("created ruleset")
    }

    pub async fn update_ruleset(
        &self,
        repo: &str,
        ruleset_id: u64,
        ruleset: &serde_json::Value,
    ) -> Result<()> {
        let path = format!("/repos/{}/{repo}/rulesets/{ruleset_id}", self.org);
        response::expect_empty(self.put_json(&path, ruleset).await?, "PUT", &path).await
    }

    pub async fn delete_ruleset(&self, repo: &str, ruleset_id: u64) -> Result<()> {
        let path = format!("/repos/{}/{repo}/rulesets/{ruleset_id}", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn get_team_id(&self, team_slug: &str) -> Result<u64> {
        self.cached_org_keyed(
            |lookups| &lookups.team_ids,
            team_slug,
            self.fetch_team_id(team_slug),
        )
        .await
    }

    async fn fetch_team_id(&self, team_slug: &str) -> Result<u64> {
        #[derive(Deserialize)]
        struct TeamIdResponse {
            id: u64,
        }

        let path = format!("/orgs/{}/teams/{team_slug}", self.org);
        Ok(
            response::expect_json::<TeamIdResponse>(self.get(&path).await?, "GET", &path)
                .await
                .read_ctx("team")?
                .id,
        )
    }

    pub async fn get_user_by_login(&self, login: &str) -> Result<GitHubUser> {
        self.cached_org_keyed(
            |lookups| &lookups.users,
            login,
            self.fetch_user_by_login(login),
        )
        .await
    }

    async fn fetch_user_by_login(&self, login: &str) -> Result<GitHubUser> {
        let path = format!("/users/{login}");
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .read_ctx("user")
    }

    pub async fn list_ruleset_repo_collaborators(
        &self,
        repo: &str,
    ) -> Result<Vec<RulesetRepositoryCollaborator>> {
        pagination::collect_paginated(self, |page| {
            format!(
                "/repos/{}/{repo}/collaborators?affiliation=all&per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .read_ctx("repository collaborators")
    }

    pub async fn list_ruleset_custom_repository_roles(
        &self,
    ) -> Result<Vec<RulesetCustomRepositoryRole>> {
        self.list_custom_repository_roles()
            .await
            .map(|roles| {
                roles
                    .into_iter()
                    .map(|role| RulesetCustomRepositoryRole {
                        id: role.id,
                        name: role.name,
                    })
                    .collect()
            })
            .context("Failed to list custom repository roles for ruleset actor resolution")
    }

    pub async fn list_org_installations(&self) -> Result<Vec<InstalledApp>> {
        self.cached_org(
            |lookups| &lookups.installations,
            self.fetch_org_installations(),
        )
        .await
    }

    async fn fetch_org_installations(&self) -> Result<Vec<InstalledApp>> {
        pagination::collect_paginated_wrapped(
            self,
            pagination::PAGE_SIZE,
            "organization installations",
            |page| {
                format!(
                    "/orgs/{}/installations?per_page={}&page={}",
                    self.org, page.per_page, page.number
                )
            },
            |payload: InstallationsResponse| pagination::WrappedPage {
                items: payload.installations,
                total_count: payload.total_count,
            },
        )
        .await
    }
}
