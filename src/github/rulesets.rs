use anyhow::{Context, Result};
use serde::Deserialize;

use super::Client;

#[derive(Debug, Deserialize)]
pub struct Ruleset {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct RulesetRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
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

    /// Get details for a specific ruleset.
    pub async fn get_ruleset(&self, repo: &str, ruleset_id: u64) -> Result<RulesetDetail> {
        let resp = self
            .get(&format!("/repos/{}/{repo}/rulesets/{ruleset_id}", self.org))
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get ruleset {ruleset_id} for {repo} (HTTP {status}): {body}");
        }

        resp.json()
            .await
            .context("Failed to parse ruleset detail response")
    }

    /// Create a new ruleset for a repository.
    pub async fn create_ruleset(
        &self,
        repo: &str,
        ruleset: &serde_json::Value,
    ) -> Result<RulesetDetail> {
        let resp = self
            .post_json(&format!("/repos/{}/{repo}/rulesets", self.org), ruleset)
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to create ruleset for {repo} (HTTP {status}): {body}");
        }

        resp.json()
            .await
            .context("Failed to parse created ruleset response")
    }

    /// Update an existing ruleset.
    pub async fn update_ruleset(
        &self,
        repo: &str,
        ruleset_id: u64,
        ruleset: &serde_json::Value,
    ) -> Result<()> {
        let resp = self
            .put_json(
                &format!("/repos/{}/{repo}/rulesets/{ruleset_id}", self.org),
                ruleset,
            )
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to update ruleset {ruleset_id} for {repo} (HTTP {status}): {body}"
            );
        }

        Ok(())
    }

    /// Delete a ruleset from a repository.
    pub async fn delete_ruleset(&self, repo: &str, ruleset_id: u64) -> Result<()> {
        let resp = self
            .delete(&format!("/repos/{}/{repo}/rulesets/{ruleset_id}", self.org))
            .await?;

        let status = resp.status();
        if !status.is_success() && status.as_u16() != 204 {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to delete ruleset {ruleset_id} for {repo} (HTTP {status}): {body}"
            );
        }

        Ok(())
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
