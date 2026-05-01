use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::manifest::SecurityCheck;

use super::Client;
use super::repos::Repository;

/// Current security state of a repository.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// Extract secret-scanning fields from a pre-fetched `security_and_analysis` JSON value.
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
    /// Read the current security state of a repository.
    ///
    /// This makes 3 HTTP requests. Prefer [`get_security_state_with_repo_data`]
    /// when you already have the repo JSON (e.g., from a repo listing) to skip
    /// the extra GET /repos/{org}/{repo} call.
    pub async fn get_security_state(&self, repo: &str) -> Result<SecurityState> {
        self.get_security_state_with_repo_data(repo, None).await
    }

    /// Read the current security state, optionally using pre-fetched repo data.
    ///
    /// When `repo_data` contains a `security_and_analysis` object, secret scanning
    /// fields are extracted from it and the separate GET /repos/{org}/{repo} call
    /// is skipped, reducing API calls from 3 to 2.
    ///
    /// The two remaining Dependabot calls are executed concurrently.
    pub async fn get_security_state_with_repo_data(
        &self,
        repo: &str,
        repo_data: Option<&serde_json::Value>,
    ) -> Result<SecurityState> {
        let mut state = SecurityState::default();

        // --- Dependabot calls (concurrent) ---
        let alerts_path = format!("/repos/{}/{repo}/vulnerability-alerts", self.org);
        let fixes_path = format!("/repos/{}/{repo}/automated-security-fixes", self.org);

        let (alerts_result, fixes_result) =
            tokio::join!(self.get(&alerts_path), self.get(&fixes_path));

        // Dependabot alerts (vulnerability-alerts): 204 = enabled
        match alerts_result {
            Ok(resp) => state.dependabot_alerts = resp.status().as_u16() == 204,
            Err(e) => tracing::warn!("Failed to check dependabot alerts for {repo}: {e}"),
        }

        // Dependabot security updates (automated-security-fixes)
        match fixes_result {
            Ok(resp) => {
                if resp.status().is_success() {
                    #[derive(Deserialize)]
                    struct AutoSecFixes {
                        enabled: bool,
                    }
                    if let Ok(body) = resp.json::<AutoSecFixes>().await {
                        state.dependabot_security_updates = body.enabled;
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to check security updates for {repo}: {e}"),
        }

        // --- Secret scanning / push protection / AI detection ---
        // Try pre-fetched data first, fall back to a fresh API call.
        let sa_from_prefetch = repo_data.and_then(|v| v.get("security_and_analysis"));

        if let Some(sa_value) = sa_from_prefetch {
            let (scanning, ai, push) = extract_scanning_from_json(sa_value);
            state.secret_scanning = scanning;
            state.secret_scanning_ai_detection = ai;
            state.push_protection = push;
        } else {
            // Fallback: fetch repo endpoint for security_and_analysis
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

    /// Run a single custom security check against a repository.
    ///
    /// Returns `true` when the check passes. Network or API errors are treated
    /// as `false` (check failed) so the TUI always gets a definitive answer.
    pub async fn run_custom_check(&self, repo: &Repository, check: &SecurityCheck) -> bool {
        match check {
            SecurityCheck::FileExists { path, .. } | SecurityCheck::WorkflowExists { path, .. } => {
                matches!(self.get_file(&repo.name, path, None).await, Ok(Some(_)))
            }
            SecurityCheck::TopicContains { topic, .. } => repo.topics.iter().any(|t| t == topic),
            SecurityCheck::BranchProtection { .. } => {
                matches!(
                    self.get_branch_protection(&repo.name, &repo.default_branch)
                        .await,
                    Ok(Some(_))
                )
            }
            SecurityCheck::DefaultBranch { expected, .. } => repo.default_branch == *expected,
        }
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

    /// Helper to build a minimal `Repository` for testing.
    fn make_repo(name: &str, default_branch: &str, topics: Vec<&str>) -> Repository {
        Repository {
            name: name.to_owned(),
            full_name: format!("org/{name}"),
            archived: false,
            default_branch: default_branch.to_owned(),
            description: None,
            visibility: "private".to_owned(),
            language: None,
            security_and_analysis: None,
            topics: topics.into_iter().map(String::from).collect(),
        }
    }

    #[tokio::test]
    async fn custom_check_default_branch_match() {
        let client = Client::new_for_test("org", "http://unused");
        let repo = make_repo("my-repo", "main", vec![]);
        let check = SecurityCheck::DefaultBranch {
            name: "Main Branch".into(),
            expected: "main".into(),
        };
        assert!(client.run_custom_check(&repo, &check).await);
    }

    #[tokio::test]
    async fn custom_check_default_branch_mismatch() {
        let client = Client::new_for_test("org", "http://unused");
        let repo = make_repo("my-repo", "master", vec![]);
        let check = SecurityCheck::DefaultBranch {
            name: "Main Branch".into(),
            expected: "main".into(),
        };
        assert!(!client.run_custom_check(&repo, &check).await);
    }

    #[tokio::test]
    async fn custom_check_topic_contains_found() {
        let client = Client::new_for_test("org", "http://unused");
        let repo = make_repo("my-repo", "main", vec!["ward-managed", "backend"]);
        let check = SecurityCheck::TopicContains {
            name: "Managed".into(),
            topic: "ward-managed".into(),
        };
        assert!(client.run_custom_check(&repo, &check).await);
    }

    #[tokio::test]
    async fn custom_check_topic_contains_not_found() {
        let client = Client::new_for_test("org", "http://unused");
        let repo = make_repo("my-repo", "main", vec!["backend"]);
        let check = SecurityCheck::TopicContains {
            name: "Managed".into(),
            topic: "ward-managed".into(),
        };
        assert!(!client.run_custom_check(&repo, &check).await);
    }

    #[tokio::test]
    async fn custom_check_topic_contains_empty_topics() {
        let client = Client::new_for_test("org", "http://unused");
        let repo = make_repo("my-repo", "main", vec![]);
        let check = SecurityCheck::TopicContains {
            name: "Managed".into(),
            topic: "ward-managed".into(),
        };
        assert!(!client.run_custom_check(&repo, &check).await);
    }
}
