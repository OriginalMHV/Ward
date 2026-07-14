use anyhow::{Context, Result};
use serde::Deserialize;

use super::Client;
use super::response;

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub html_url: String,
    pub state: String,
    pub title: String,
    pub head: PullRequestHead,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestHead {
    #[serde(rename = "ref")]
    pub branch: String,
}

impl Client {
    /// Create a pull request. Returns the created PR.
    pub async fn create_pull_request(
        &self,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
        reviewers: &[String],
    ) -> Result<PullRequest> {
        // Check for existing PR from the same branch
        if let Some(existing) = self.find_open_pull_request(repo, head).await? {
            tracing::info!(
                "PR already exists for {head} in {repo}: {}",
                existing.html_url
            );
            return Ok(existing);
        }

        let pr_body = serde_json::json!({
            "title": title,
            "body": body,
            "head": head,
            "base": base,
        });

        let path = format!("/repos/{}/{repo}/pulls", self.org);
        let pr: PullRequest =
            response::expect_json(self.post_json(&path, &pr_body).await?, "POST", &path)
                .await
                .context("Failed to parse PR response")?;

        // Request reviewers (best-effort)
        if !reviewers.is_empty() {
            let review_body = serde_json::json!({
                "reviewers": reviewers,
            });

            let _ = self
                .post_json(
                    &format!(
                        "/repos/{}/{repo}/pulls/{}/requested_reviewers",
                        self.org, pr.number
                    ),
                    &review_body,
                )
                .await;
        }

        Ok(pr)
    }

    /// Find an open PR from the given branch.
    pub(crate) async fn find_open_pull_request(
        &self,
        repo: &str,
        head_branch: &str,
    ) -> Result<Option<PullRequest>> {
        let path = format!(
            "/repos/{org}/{repo}/pulls?state=open&head={org}:{head_branch}",
            org = self.org,
        );
        let prs: Vec<PullRequest> =
            response::expect_json(self.get(&path).await?, "GET", &path).await?;
        Ok(prs.into_iter().next())
    }
}
