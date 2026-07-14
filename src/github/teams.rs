use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::manifest::TeamAccess;

use super::Client;
use super::actions::{ReadOutcome, classify_read};
use super::environments::encode_path_segment;
use super::pagination;
use super::response;

#[derive(Debug)]
pub struct Team {
    pub id: u64,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permission: String,
    pub privacy: String,
}

impl<'de> Deserialize<'de> for Team {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct TeamApi {
            id: u64,
            name: String,
            slug: String,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            permission: String,
            #[serde(default)]
            role_name: Option<String>,
            #[serde(default)]
            privacy: String,
        }

        let team = TeamApi::deserialize(deserializer)?;
        Ok(Self {
            id: team.id,
            name: team.name,
            slug: team.slug,
            description: team.description,
            permission: team.role_name.unwrap_or(team.permission),
            privacy: team.privacy,
        })
    }
}

impl Team {
    pub fn effective_permission(&self) -> &str {
        &self.permission
    }
}

impl From<&Team> for TeamAccess {
    fn from(value: &Team) -> Self {
        Self {
            slug: value.slug.clone(),
            permission: value.effective_permission().to_owned(),
        }
    }
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
        pagination::collect_paginated(self, |page| {
            format!(
                "/orgs/{}/teams?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse org teams response")
    }

    pub async fn list_org_teams_checked(&self) -> Result<ReadOutcome<Vec<Team>>> {
        collect_paginated_checked(self, |page| {
            format!(
                "/orgs/{}/teams?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse org teams response")
    }

    /// List teams that have access to a repository.
    pub async fn list_repo_teams(&self, repo: &str) -> Result<Vec<Team>> {
        pagination::collect_paginated(self, |page| {
            format!(
                "/repos/{}/{repo}/teams?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse repo teams response")
    }

    pub async fn list_repo_teams_checked(&self, repo: &str) -> Result<ReadOutcome<Vec<Team>>> {
        collect_paginated_checked(self, |page| {
            format!(
                "/repos/{}/{repo}/teams?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
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
        let encoded_team_slug = encode_path_segment(team_slug);
        let path = format!(
            "/orgs/{}/teams/{encoded_team_slug}/repos/{}/{repo}",
            self.org, self.org
        );
        response::expect_empty(self.put_json(&path, &body).await?, "PUT", &path).await
    }

    /// Remove a team's access from a repository.
    pub async fn remove_team_from_repo(&self, repo: &str, team_slug: &str) -> Result<()> {
        let encoded_team_slug = encode_path_segment(team_slug);
        let path = format!(
            "/orgs/{}/teams/{encoded_team_slug}/repos/{}/{repo}",
            self.org, self.org
        );
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }
}

async fn collect_paginated_checked<T, F>(
    client: &Client,
    mut build_path: F,
) -> Result<ReadOutcome<Vec<T>>>
where
    T: DeserializeOwned,
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
