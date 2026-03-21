use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::Client;

/// Current security state of a repository.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SecurityState {
    pub dependabot_alerts: bool,
    pub dependabot_security_updates: bool,
    pub secret_scanning: bool,
    pub secret_scanning_ai_detection: bool,
    pub push_protection: bool,
}

#[derive(Debug, Deserialize)]
struct RepoSecurityResponse {
    security_and_analysis: Option<SecurityAndAnalysis>,
}

#[derive(Debug, Deserialize)]
struct SecurityAndAnalysis {
    secret_scanning: Option<FeatureStatus>,
    secret_scanning_ai_detection: Option<FeatureStatus>,
    secret_scanning_push_protection: Option<FeatureStatus>,
}

#[derive(Debug, Deserialize)]
struct FeatureStatus {
    status: String,
}

impl Client {
    /// Read the current security state of a repository.
    pub async fn get_security_state(&self, repo: &str) -> Result<SecurityState> {
        let mut state = SecurityState::default();

        // Dependabot alerts (vulnerability-alerts)
        let resp = self
            .get(&format!("/repos/{}/{repo}/vulnerability-alerts", self.org))
            .await?;
        state.dependabot_alerts = resp.status().as_u16() == 204;

        // Dependabot security updates (automated-security-fixes)
        let resp = self
            .get(&format!(
                "/repos/{}/{repo}/automated-security-fixes",
                self.org
            ))
            .await?;

        if resp.status().is_success() {
            #[derive(Deserialize)]
            struct AutoSecFixes {
                enabled: bool,
            }
            if let Ok(body) = resp.json::<AutoSecFixes>().await {
                state.dependabot_security_updates = body.enabled;
            }
        }

        // Secret scanning, AI detection, push protection (from repo settings)
        let resp = self.get(&format!("/repos/{}/{repo}", self.org)).await?;

        if resp.status().is_success()
            && let Ok(body) = resp.json::<RepoSecurityResponse>().await
            && let Some(sa) = body.security_and_analysis
        {
            state.secret_scanning = sa
                .secret_scanning
                .as_ref()
                .is_some_and(|f| f.status == "enabled");
            state.secret_scanning_ai_detection = sa
                .secret_scanning_ai_detection
                .as_ref()
                .is_some_and(|f| f.status == "enabled");
            state.push_protection = sa
                .secret_scanning_push_protection
                .as_ref()
                .is_some_and(|f| f.status == "enabled");
        }

        Ok(state)
    }

    /// Enable vulnerability alerts (Dependabot alerts) for a repo.
    pub async fn enable_dependabot_alerts(&self, repo: &str) -> Result<()> {
        let resp = self
            .put(&format!("/repos/{}/{repo}/vulnerability-alerts", self.org))
            .await?;

        ensure_success(resp, "enable Dependabot alerts", repo).await
    }

    /// Enable automated security fixes (Dependabot security updates) for a repo.
    pub async fn enable_dependabot_security_updates(&self, repo: &str) -> Result<()> {
        let resp = self
            .put(&format!(
                "/repos/{}/{repo}/automated-security-fixes",
                self.org
            ))
            .await?;

        ensure_success(resp, "enable Dependabot security updates", repo).await
    }

    /// Enable secret scanning, AI detection, and/or push protection.
    pub async fn set_security_features(
        &self,
        repo: &str,
        secret_scanning: bool,
        ai_detection: bool,
        push_protection: bool,
    ) -> Result<()> {
        let body = serde_json::json!({
            "security_and_analysis": {
                "secret_scanning": {
                    "status": if secret_scanning { "enabled" } else { "disabled" }
                },
                "secret_scanning_ai_detection": {
                    "status": if ai_detection { "enabled" } else { "disabled" }
                },
                "secret_scanning_push_protection": {
                    "status": if push_protection { "enabled" } else { "disabled" }
                }
            }
        });

        let resp = self
            .patch_json(&format!("/repos/{}/{repo}", self.org), &body)
            .await?;

        ensure_success(resp, "set security features", repo).await
    }
}

async fn ensure_success(resp: reqwest::Response, action: &str, repo: &str) -> Result<()> {
    let status = resp.status();
    if status.is_success() || status.as_u16() == 204 {
        Ok(())
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Failed to {action} for {repo} (HTTP {status}): {body}")
    }
}
