use anyhow::{Context, Result};
use serde::Deserialize;

use super::Client;

#[derive(Debug, Deserialize)]
pub struct Team {
    pub id: u64,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub permission: String,
    #[serde(default)]
    pub privacy: String,
}

#[derive(Debug, Deserialize)]
pub struct TeamMember {
    pub login: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct TeamRepoPermission {
    pub team_slug: String,
    pub permission: String,
}

impl Client {
    /// List all teams in the organization, handling pagination.
    pub async fn list_org_teams(&self) -> Result<Vec<Team>> {
        let mut all_teams = Vec::new();
        let mut page = 1u32;

        loop {
            let resp = self
                .get(&format!(
                    "/orgs/{}/teams?per_page=100&page={page}",
                    self.org
                ))
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Failed to list org teams (HTTP {status}): {body}");
            }

            let teams: Vec<Team> = resp
                .json()
                .await
                .context("Failed to parse org teams response")?;

            if teams.is_empty() {
                break;
            }

            all_teams.extend(teams);
            page += 1;
        }

        Ok(all_teams)
    }

    /// List teams that have access to a repository.
    pub async fn list_repo_teams(&self, repo: &str) -> Result<Vec<Team>> {
        let resp = self
            .get(&format!("/repos/{}/{repo}/teams", self.org))
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to list teams for {repo} (HTTP {status}): {body}");
        }

        resp.json()
            .await
            .context("Failed to parse repo teams response")
    }

    /// Add or update a team's access to a repository.
    pub async fn add_team_to_repo(
        &self,
        repo: &str,
        team_slug: &str,
        permission: &str,
    ) -> Result<()> {
        let body = serde_json::json!({ "permission": permission });
        let resp = self
            .put_json(
                &format!(
                    "/orgs/{}/teams/{team_slug}/repos/{}/{repo}",
                    self.org, self.org
                ),
                &body,
            )
            .await?;

        let status = resp.status();
        if !status.is_success() && status.as_u16() != 204 {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to add team {team_slug} to {repo} (HTTP {status}): {body}");
        }

        Ok(())
    }

    /// Remove a team's access from a repository.
    pub async fn remove_team_from_repo(&self, repo: &str, team_slug: &str) -> Result<()> {
        let resp = self
            .delete(&format!(
                "/orgs/{}/teams/{team_slug}/repos/{}/{repo}",
                self.org, self.org
            ))
            .await?;

        let status = resp.status();
        if !status.is_success() && status.as_u16() != 204 {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to remove team {team_slug} from {repo} (HTTP {status}): {body}");
        }

        Ok(())
    }
}
