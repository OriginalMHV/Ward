use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Client;
use super::actions::{ReadOutcome, classify_read};
use super::pagination;
use super::response;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityFeatureStatus {
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedBypassReviewer {
    pub reviewer_id: u64,
    pub reviewer_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedBypassOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<DelegatedBypassReviewer>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityAndAnalysisState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advanced_security: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_security: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependabot_security_updates: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_ai_detection: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_push_protection: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_validity_checks: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_non_provider_patterns: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_alert_dismissal_options: Option<DelegatedBypassOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_alert_dismissal: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_bypass: Option<SecurityFeatureStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_bypass_options: Option<DelegatedBypassOptions>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeqlDefaultSetupState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeSecurityConfiguration {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub target_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryCodeSecurityConfiguration {
    #[serde(default)]
    pub status: String,
    pub configuration: CodeSecurityConfiguration,
}

/// Current security state of a repository used by legacy security commands.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityState {
    pub dependabot_alerts: bool,
    pub dependabot_security_updates: bool,
    pub secret_scanning: bool,
    pub secret_scanning_ai_detection: bool,
    pub push_protection: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RepositorySecurityBaseline {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub security_and_analysis: Option<SecurityAndAnalysisState>,
}

#[derive(Debug, Deserialize)]
struct EnabledState {
    enabled: bool,
}

fn status_is_enabled(status: Option<&SecurityFeatureStatus>) -> bool {
    status.is_some_and(|value| value.status == "enabled")
}

fn security_status(status: bool) -> serde_json::Value {
    json!({
        "status": if status { "enabled" } else { "disabled" }
    })
}

/// Extract secret-scanning fields from a pre-fetched `security_and_analysis` JSON value.
#[cfg(test)]
fn extract_scanning_from_json(sa_value: &serde_json::Value) -> (bool, bool, bool) {
    let enabled = |key: &str| -> bool {
        sa_value
            .get(key)
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == "enabled")
    };

    (
        enabled("secret_scanning"),
        enabled("secret_scanning_ai_detection"),
        enabled("secret_scanning_push_protection"),
    )
}

impl Client {
    pub async fn get_repository_security_baseline(
        &self,
        repo: &str,
    ) -> Result<RepositorySecurityBaseline> {
        let path = format!("/repos/{}/{repo}", self.org);
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse repository security baseline response")
    }

    pub async fn get_repository_security_and_analysis_with_repo_data(
        &self,
        repo: &str,
        repo_data: Option<&serde_json::Value>,
    ) -> Result<Option<SecurityAndAnalysisState>> {
        if let Some(repo_data) = repo_data {
            return Self::parse_prefetched_security_and_analysis(repo_data);
        }

        Ok(self
            .get_repository_security_baseline(repo)
            .await?
            .security_and_analysis)
    }

    pub async fn update_repository_security_and_analysis(
        &self,
        repo: &str,
        security_and_analysis: &serde_json::Value,
    ) -> Result<()> {
        let path = format!("/repos/{}/{repo}", self.org);
        let body = json!({
            "security_and_analysis": security_and_analysis,
        });
        response::expect_empty(self.patch_json(&path, &body).await?, "PATCH", &path).await
    }

    fn parse_prefetched_security_and_analysis(
        repo_data: &serde_json::Value,
    ) -> Result<Option<SecurityAndAnalysisState>> {
        let value = repo_data.get("security_and_analysis").unwrap_or(repo_data);
        if value.is_null() {
            return Ok(None);
        }

        serde_json::from_value(value.clone())
            .map(Some)
            .context("Failed to parse pre-fetched security_and_analysis data")
    }

    pub async fn read_private_vulnerability_reporting_status(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<bool>> {
        let path = format!("/repos/{}/{repo}/private-vulnerability-reporting", self.org);
        match classify_read::<EnabledState>(self.get(&path).await?, "GET", &path, true).await? {
            ReadOutcome::Available(state) => Ok(ReadOutcome::Available(state.enabled)),
            ReadOutcome::NotApplicable(reason) => Ok(ReadOutcome::NotApplicable(reason)),
            ReadOutcome::PermissionDenied(reason) => Ok(ReadOutcome::PermissionDenied(reason)),
            ReadOutcome::Unavailable(reason) => Ok(ReadOutcome::Unavailable(reason)),
        }
    }

    pub async fn set_private_vulnerability_reporting(
        &self,
        repo: &str,
        enabled: bool,
    ) -> Result<()> {
        let path = format!("/repos/{}/{repo}/private-vulnerability-reporting", self.org);
        let response = if enabled {
            self.put(&path).await?
        } else {
            self.delete(&path).await?
        };
        response::expect_empty(response, if enabled { "PUT" } else { "DELETE" }, &path).await
    }

    pub async fn read_codeql_default_setup(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<CodeqlDefaultSetupState>> {
        let path = format!("/repos/{}/{repo}/code-scanning/default-setup", self.org);
        classify_read(self.get(&path).await?, "GET", &path, false)
            .await
            .context("Failed to parse CodeQL default setup response")
    }

    pub async fn update_codeql_default_setup(
        &self,
        repo: &str,
        body: &CodeqlDefaultSetupState,
    ) -> Result<Option<CodeqlDefaultSetupState>> {
        let path = format!("/repos/{}/{repo}/code-scanning/default-setup", self.org);
        response::optional_json(self.patch_json(&path, body).await?, "PATCH", &path)
            .await
            .context("Failed to parse CodeQL default setup update response")
    }

    pub async fn list_code_security_configurations(
        &self,
    ) -> Result<Vec<CodeSecurityConfiguration>> {
        pagination::collect_paginated(self, |page| {
            format!(
                "/orgs/{}/code-security/configurations?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse code security configurations response")
    }

    pub async fn read_repository_code_security_configuration(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<RepositoryCodeSecurityConfiguration>> {
        let path = format!("/repos/{}/{repo}/code-security-configuration", self.org);
        classify_read(self.get(&path).await?, "GET", &path, false)
            .await
            .context("Failed to parse repository code security configuration response")
    }

    pub async fn attach_code_security_configuration(
        &self,
        configuration_id: u64,
        repository_id: u64,
    ) -> Result<()> {
        let path = format!(
            "/orgs/{}/code-security/configurations/{configuration_id}/attach",
            self.org
        );
        let body = json!({
            "scope": "selected",
            "selected_repository_ids": [repository_id],
        });
        response::expect_empty(self.post_json(&path, &body).await?, "POST", &path).await
    }

    pub async fn detach_code_security_configurations(&self, repository_ids: &[u64]) -> Result<()> {
        let path = format!("/orgs/{}/code-security/configurations/detach", self.org);
        let body = json!({
            "selected_repository_ids": repository_ids,
        });
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .http
            .delete(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("DELETE {url} failed"))?;
        response::expect_empty(response, "DELETE", &path).await
    }

    /// Read the current security state of a repository.
    pub async fn get_security_state(&self, repo: &str) -> Result<SecurityState> {
        self.get_security_state_with_repo_data(repo, None).await
    }

    /// Read the current security state, optionally using pre-fetched repo data.
    pub async fn get_security_state_with_repo_data(
        &self,
        repo: &str,
        repo_data: Option<&serde_json::Value>,
    ) -> Result<SecurityState> {
        let alerts_path = format!("/repos/{}/{repo}/vulnerability-alerts", self.org);
        let fixes_path = format!("/repos/{}/{repo}/automated-security-fixes", self.org);

        let (alerts_result, fixes_result, repo_security_result) = tokio::join!(
            self.get(&alerts_path),
            self.get(&fixes_path),
            self.get_repository_security_and_analysis_with_repo_data(repo, repo_data)
        );

        let mut state = SecurityState::default();

        match alerts_result {
            Ok(response) => state.dependabot_alerts = response.status().as_u16() == 204,
            Err(error) => tracing::warn!("Failed to check dependabot alerts for {repo}: {error}"),
        }

        match fixes_result {
            Ok(response) => {
                if response.status().is_success() {
                    #[derive(Deserialize)]
                    struct AutoSecFixes {
                        enabled: bool,
                    }
                    if let Ok(body) = response.json::<AutoSecFixes>().await {
                        state.dependabot_security_updates = body.enabled;
                    }
                }
            }
            Err(error) => {
                tracing::warn!("Failed to check security updates for {repo}: {error}");
            }
        }

        match repo_security_result {
            Ok(Some(security)) => {
                state.dependabot_security_updates = state.dependabot_security_updates
                    || status_is_enabled(security.dependabot_security_updates.as_ref());
                state.secret_scanning = status_is_enabled(security.secret_scanning.as_ref());
                state.secret_scanning_ai_detection =
                    status_is_enabled(security.secret_scanning_ai_detection.as_ref());
                state.push_protection =
                    status_is_enabled(security.secret_scanning_push_protection.as_ref());
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!("Failed to inspect repository security settings for {repo}: {error}")
            }
        }

        Ok(state)
    }

    pub async fn read_dependabot_alerts_state(&self, repo: &str) -> Result<ReadOutcome<bool>> {
        let path = format!("/repos/{}/{repo}/vulnerability-alerts", self.org);
        Ok(
            match response::classify_empty(self.get(&path).await?, "GET", &path).await? {
                response::ClassifiedResponse::Success(())
                | response::ClassifiedResponse::NoContent => ReadOutcome::Available(true),
                response::ClassifiedResponse::Forbidden(error) => {
                    ReadOutcome::PermissionDenied(error.to_string())
                }
                response::ClassifiedResponse::Unprocessable(error) => {
                    ReadOutcome::Unavailable(error.to_string())
                }
                response::ClassifiedResponse::NotFound(error)
                | response::ClassifiedResponse::Other(error) => {
                    ReadOutcome::Unavailable(error.to_string())
                }
            },
        )
    }

    pub async fn read_dependabot_security_updates_state(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<bool>> {
        let path = format!("/repos/{}/{repo}/automated-security-fixes", self.org);
        match classify_read::<EnabledState>(self.get(&path).await?, "GET", &path, false).await? {
            ReadOutcome::Available(state) => Ok(ReadOutcome::Available(state.enabled)),
            ReadOutcome::NotApplicable(reason) => Ok(ReadOutcome::NotApplicable(reason)),
            ReadOutcome::PermissionDenied(reason) => Ok(ReadOutcome::PermissionDenied(reason)),
            ReadOutcome::Unavailable(reason) => Ok(ReadOutcome::Unavailable(reason)),
        }
    }

    pub async fn enable_dependabot_alerts(&self, repo: &str) -> Result<()> {
        let path = format!("/repos/{}/{repo}/vulnerability-alerts", self.org);
        response::expect_empty(self.put(&path).await?, "PUT", &path).await
    }

    pub async fn disable_dependabot_alerts(&self, repo: &str) -> Result<()> {
        let path = format!("/repos/{}/{repo}/vulnerability-alerts", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn enable_dependabot_security_updates(&self, repo: &str) -> Result<()> {
        let path = format!("/repos/{}/{repo}/automated-security-fixes", self.org);
        response::expect_empty(self.put(&path).await?, "PUT", &path).await
    }

    pub async fn disable_dependabot_security_updates(&self, repo: &str) -> Result<()> {
        let path = format!("/repos/{}/{repo}/automated-security-fixes", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn set_security_features(
        &self,
        repo: &str,
        secret_scanning: bool,
        ai_detection: bool,
        push_protection: bool,
    ) -> Result<()> {
        let body = json!({
            "secret_scanning": security_status(secret_scanning),
            "secret_scanning_ai_detection": security_status(ai_detection),
            "secret_scanning_push_protection": security_status(push_protection),
            "advanced_security": security_status(secret_scanning || ai_detection || push_protection),
        });
        self.update_repository_security_and_analysis(repo, &body)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_all_enabled() {
        let sa = json!({
            "secret_scanning": { "status": "enabled" },
            "secret_scanning_ai_detection": { "status": "enabled" },
            "secret_scanning_push_protection": { "status": "enabled" }
        });

        let (scanning, ai, push) = extract_scanning_from_json(&sa);
        assert!(scanning);
        assert!(ai);
        assert!(push);
    }

    #[test]
    fn extract_all_disabled() {
        let sa = json!({
            "secret_scanning": { "status": "disabled" },
            "secret_scanning_ai_detection": { "status": "disabled" },
            "secret_scanning_push_protection": { "status": "disabled" }
        });

        let (scanning, ai, push) = extract_scanning_from_json(&sa);
        assert!(!scanning);
        assert!(!ai);
        assert!(!push);
    }

    #[test]
    fn extract_mixed_states() {
        let sa = json!({
            "secret_scanning": { "status": "enabled" },
            "secret_scanning_ai_detection": { "status": "disabled" },
            "secret_scanning_push_protection": { "status": "enabled" }
        });

        let (scanning, ai, push) = extract_scanning_from_json(&sa);
        assert!(scanning);
        assert!(!ai);
        assert!(push);
    }

    #[test]
    fn extract_missing_fields_default_to_false() {
        let sa = json!({});

        let (scanning, ai, push) = extract_scanning_from_json(&sa);
        assert!(!scanning);
        assert!(!ai);
        assert!(!push);
    }

    #[test]
    fn extract_null_value() {
        let sa = json!({
            "secret_scanning": null,
            "secret_scanning_ai_detection": { "status": "enabled" },
            "secret_scanning_push_protection": null
        });

        let (scanning, ai, push) = extract_scanning_from_json(&sa);
        assert!(!scanning);
        assert!(ai);
        assert!(!push);
    }

    #[test]
    fn status_helper_detects_enabled() {
        assert!(status_is_enabled(Some(&SecurityFeatureStatus {
            status: "enabled".to_owned(),
        })));
        assert!(!status_is_enabled(Some(&SecurityFeatureStatus {
            status: "disabled".to_owned(),
        })));
        assert!(!status_is_enabled(None));
    }

    #[test]
    fn parses_security_from_full_prefetched_repository_payload() {
        let payload = json!({
            "name": "example",
            "security_and_analysis": {
                "secret_scanning": { "status": "enabled" }
            }
        });

        let security = Client::parse_prefetched_security_and_analysis(&payload)
            .unwrap()
            .unwrap();

        assert!(status_is_enabled(security.secret_scanning.as_ref()));
    }

    #[test]
    fn parses_direct_prefetched_security_payload() {
        let payload = json!({
            "secret_scanning_push_protection": { "status": "enabled" }
        });

        let security = Client::parse_prefetched_security_and_analysis(&payload)
            .unwrap()
            .unwrap();

        assert!(status_is_enabled(
            security.secret_scanning_push_protection.as_ref()
        ));
    }
}
