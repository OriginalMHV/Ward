use anyhow::Result;
use serde::Deserialize;

use super::Client;

#[derive(Debug, Deserialize)]
pub struct Ruleset {
    pub id: u64,
    pub name: String,
}

impl Client {
    /// List rulesets for a repository.
    pub async fn list_rulesets(&self, repo: &str) -> Result<Vec<Ruleset>> {
        let resp = self
            .get(&format!("/repos/{}/{repo}/rulesets", self.org))
            .await?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        Ok(resp.json().await.unwrap_or_default())
    }

    /// Create a Copilot code review ruleset.
    pub async fn create_copilot_review_ruleset(&self, repo: &str) -> Result<()> {
        let existing = self.list_rulesets(repo).await?;
        if existing.iter().any(|r| r.name == "Copilot Code Review") {
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

        let resp = self
            .post_json(&format!("/repos/{}/{repo}/rulesets", self.org), &body)
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to create Copilot review ruleset for {repo} (HTTP {status}): {body}"
            );
        }

        Ok(())
    }
}
