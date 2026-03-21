use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::Client;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BranchProtectionState {
    pub required_pull_request_reviews: bool,
    pub required_approving_review_count: u32,
    pub dismiss_stale_reviews: bool,
    pub require_code_owner_reviews: bool,
    pub required_status_checks: bool,
    pub strict_status_checks: bool,
    pub enforce_admins: bool,
    pub required_linear_history: bool,
    pub allow_force_pushes: bool,
    pub allow_deletions: bool,
}

#[derive(Debug, Deserialize)]
struct BranchProtectionResponse {
    #[serde(default)]
    required_pull_request_reviews: Option<PullRequestReviewConfig>,
    #[serde(default)]
    required_status_checks: Option<StatusChecksConfig>,
    #[serde(default)]
    enforce_admins: Option<EnforceAdmins>,
    #[serde(default)]
    required_linear_history: Option<EnabledFlag>,
    #[serde(default)]
    allow_force_pushes: Option<EnabledFlag>,
    #[serde(default)]
    allow_deletions: Option<EnabledFlag>,
}

#[derive(Debug, Deserialize)]
struct PullRequestReviewConfig {
    #[serde(default)]
    required_approving_review_count: u32,
    #[serde(default)]
    dismiss_stale_reviews: bool,
    #[serde(default)]
    require_code_owner_reviews: bool,
}

#[derive(Debug, Deserialize)]
struct StatusChecksConfig {
    #[serde(default)]
    strict: bool,
}

#[derive(Debug, Deserialize)]
struct EnforceAdmins {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct EnabledFlag {
    enabled: bool,
}

impl Client {
    pub async fn get_branch_protection(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Option<BranchProtectionState>> {
        let resp = self
            .get(&format!(
                "/repos/{}/{repo}/branches/{branch}/protection",
                self.org
            ))
            .await?;

        if resp.status().as_u16() == 404 {
            return Ok(None);
        }

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to get branch protection for {repo}/{branch} (HTTP {status}): {body}"
            );
        }

        let body: BranchProtectionResponse = resp.json().await?;

        let state = BranchProtectionState {
            required_pull_request_reviews: body.required_pull_request_reviews.is_some(),
            required_approving_review_count: body
                .required_pull_request_reviews
                .as_ref()
                .map(|r| r.required_approving_review_count)
                .unwrap_or(0),
            dismiss_stale_reviews: body
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|r| r.dismiss_stale_reviews),
            require_code_owner_reviews: body
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|r| r.require_code_owner_reviews),
            required_status_checks: body.required_status_checks.is_some(),
            strict_status_checks: body
                .required_status_checks
                .as_ref()
                .is_some_and(|r| r.strict),
            enforce_admins: body.enforce_admins.as_ref().is_some_and(|e| e.enabled),
            required_linear_history: body
                .required_linear_history
                .as_ref()
                .is_some_and(|f| f.enabled),
            allow_force_pushes: body.allow_force_pushes.as_ref().is_some_and(|f| f.enabled),
            allow_deletions: body.allow_deletions.as_ref().is_some_and(|f| f.enabled),
        };

        Ok(Some(state))
    }

    pub async fn update_branch_protection(
        &self,
        repo: &str,
        branch: &str,
        config: &crate::config::manifest::BranchProtectionConfig,
    ) -> Result<()> {
        let required_status_checks = if config.require_status_checks {
            serde_json::json!({
                "strict": config.strict_status_checks,
                "contexts": []
            })
        } else {
            serde_json::Value::Null
        };

        let required_pull_request_reviews = if config.enabled {
            serde_json::json!({
                "required_approving_review_count": config.required_approvals,
                "dismiss_stale_reviews": config.dismiss_stale_reviews,
                "require_code_owner_reviews": config.require_code_owner_reviews
            })
        } else {
            serde_json::Value::Null
        };

        let body = serde_json::json!({
            "required_status_checks": required_status_checks,
            "required_pull_request_reviews": required_pull_request_reviews,
            "enforce_admins": config.enforce_admins,
            "restrictions": null,
            "required_linear_history": config.required_linear_history,
            "allow_force_pushes": config.allow_force_pushes,
            "allow_deletions": config.allow_deletions
        });

        let resp = self
            .put_json(
                &format!("/repos/{}/{repo}/branches/{branch}/protection", self.org),
                &body,
            )
            .await?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to update branch protection for {repo}/{branch} (HTTP {status}): {body}"
            );
        }
    }
}
