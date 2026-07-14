use anyhow::{Context, Result};
use serde::Deserialize;

use super::Client;
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

impl Client {
    pub async fn list_rulesets(&self, repo: &str) -> Result<Vec<Ruleset>> {
        self.list_rulesets_scoped(repo, true).await
    }

    pub async fn list_repository_rulesets(&self, repo: &str) -> Result<Vec<Ruleset>> {
        self.list_rulesets_scoped(repo, false).await
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
        .context("Failed to parse rulesets response")
    }

    pub async fn get_ruleset(&self, repo: &str, ruleset_id: u64) -> Result<RulesetDetail> {
        let path = format!("/repos/{}/{repo}/rulesets/{ruleset_id}", self.org);
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse ruleset detail response")
    }

    pub async fn create_ruleset(
        &self,
        repo: &str,
        ruleset: &serde_json::Value,
    ) -> Result<RulesetDetail> {
        let path = format!("/repos/{}/{repo}/rulesets", self.org);
        response::expect_json(self.post_json(&path, ruleset).await?, "POST", &path)
            .await
            .context("Failed to parse created ruleset response")
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
        #[derive(Deserialize)]
        struct TeamIdResponse {
            id: u64,
        }

        let path = format!("/orgs/{}/teams/{team_slug}", self.org);
        Ok(
            response::expect_json::<TeamIdResponse>(self.get(&path).await?, "GET", &path)
                .await
                .context("Failed to parse team response")?
                .id,
        )
    }

    pub async fn get_user_by_login(&self, login: &str) -> Result<GitHubUser> {
        let path = format!("/users/{login}");
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse user response")
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
        .context("Failed to parse repository collaborators response")
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
        let mut page = pagination::Page::default();
        let mut installations = Vec::new();

        loop {
            let path = format!(
                "/orgs/{}/installations?per_page={}&page={}",
                self.org, page.per_page, page.number
            );
            let payload: InstallationsResponse =
                response::expect_json(self.get(&path).await?, "GET", &path)
                    .await
                    .context("Failed to parse organization installations response")?;
            let item_count = payload.installations.len();
            installations.extend(payload.installations);

            if item_count < page.per_page as usize
                || payload
                    .total_count
                    .is_some_and(|total_count| installations.len() >= total_count)
            {
                break;
            }

            page = pagination::Page {
                number: page.number + 1,
                ..page
            };
        }

        Ok(installations)
    }

    pub async fn create_copilot_review_ruleset(&self, repo: &str) -> Result<()> {
        let existing = self.list_rulesets(repo).await?;
        if existing
            .iter()
            .any(|ruleset| ruleset.name == "Copilot Code Review")
        {
            tracing::info!("Copilot review ruleset already exists for {repo}");
            return Ok(());
        }

        let body = serde_json::json!({
            "name": "Copilot Code Review",
            "target": "branch",
            "enforcement": "active",
            "conditions": {
                "ref_name": {
                    "include": ["~DEFAULT_BRANCH"],
                    "exclude": []
                }
            },
            "rules": [{
                "type": "copilot_code_review",
                "parameters": {
                    "review_on_push": true
                }
            }],
            "bypass_actors": []
        });

        let path = format!("/repos/{}/{repo}/rulesets", self.org);
        response::expect_empty(self.post_json(&path, &body).await?, "POST", &path).await
    }
}
