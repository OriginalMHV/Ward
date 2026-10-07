//! Security apply and verify.

use std::time::Duration;

use anyhow::Result;

use crate::config::manifest::SecurityCategory;
use crate::github::Client;
use crate::reconcile::common::rules_issue::ReconcileIssueSeverity;

use super::*;

pub async fn apply_security_plan(
    client: &Client,
    repo: &str,
    plan: &SecurityPlan,
) -> Result<SecurityApplyResult> {
    if let Some(issue) = plan
        .issues
        .iter()
        .find(|issue| issue.severity == ReconcileIssueSeverity::Blocker)
    {
        anyhow::bail!("Security plan is blocked: {}", issue.message);
    }

    let mut applied_steps = Vec::new();

    if plan.detach_configuration {
        client
            .detach_code_security_configurations(&[plan.repository_id])
            .await?;
        applied_steps.push("detach_code_security_configuration".to_owned());
    }
    if let Some(configuration_id) = plan.attach_configuration_id {
        client
            .attach_code_security_configuration(configuration_id, plan.repository_id)
            .await?;
        applied_steps.push(format!(
            "attach_code_security_configuration:{configuration_id}"
        ));
    }
    if let Some(enabled) = plan.dependabot_alerts {
        if enabled {
            client.enable_dependabot_alerts(repo).await?;
        } else {
            client.disable_dependabot_alerts(repo).await?;
        }
        applied_steps.push("dependabot_alerts".to_owned());
    }
    if let Some(enabled) = plan.dependabot_security_updates {
        if enabled {
            client.enable_dependabot_security_updates(repo).await?;
        } else {
            client.disable_dependabot_security_updates(repo).await?;
        }
        applied_steps.push("dependabot_security_updates".to_owned());
    }
    if let Some(value) = &plan.patch_security_and_analysis {
        client
            .update_repository_security_and_analysis(repo, value)
            .await?;
        applied_steps.push("security_and_analysis".to_owned());
    }
    if let Some(enabled) = plan.private_vulnerability_reporting {
        client
            .set_private_vulnerability_reporting(repo, enabled)
            .await?;
        applied_steps.push("private_vulnerability_reporting".to_owned());
    }
    if let Some(codeql) = &plan.codeql_default_setup {
        client.update_codeql_default_setup(repo, codeql).await?;
        applied_steps.push("codeql_default_setup".to_owned());
    }

    Ok(SecurityApplyResult { applied_steps })
}

/// How long to wait for asynchronous CodeQL default setup to settle.
#[derive(Clone, Copy, Debug)]
pub struct VerifyPolicy {
    pub attempts: u32,
    pub interval: Duration,
}

impl Default for VerifyPolicy {
    fn default() -> Self {
        Self {
            attempts: 10,
            interval: Duration::from_millis(100),
        }
    }
}

impl VerifyPolicy {
    /// Test-only policy without delays. Public because integration tests are separate crates.
    pub const fn immediate_for_tests() -> Self {
        Self {
            attempts: 10,
            interval: Duration::ZERO,
        }
    }
}

pub async fn verify_security_category(
    client: &Client,
    repo: &str,
    desired: &SecurityCategory,
) -> Result<SecurityVerifyResult> {
    verify_security_category_with(client, repo, desired, VerifyPolicy::default()).await
}

pub async fn verify_security_category_with(
    client: &Client,
    repo: &str,
    desired: &SecurityCategory,
    policy: VerifyPolicy,
) -> Result<SecurityVerifyResult> {
    let mut attempt = 1;
    loop {
        let actual = collect_security_category(client, repo, Some(desired)).await?;
        let plan = plan_security_category(desired, &actual)?;
        let waiting_on_codeql = plan.codeql_default_setup.is_some()
            && actual
                .codeql_default_setup
                .as_ref()
                .and_then(|value| value.state.as_deref())
                .is_some_and(|state| {
                    matches!(state, "queued" | "pending" | "in_progress" | "configuring")
                });
        let blocked = plan
            .issues
            .iter()
            .any(|issue| issue.severity == ReconcileIssueSeverity::Blocker);
        if !plan.has_changes() && !blocked {
            return Ok(SecurityVerifyResult {
                matches: true,
                plan,
            });
        }
        if waiting_on_codeql && attempt < policy.attempts {
            tokio::time::sleep(policy.interval).await;
            attempt += 1;
            continue;
        }
        return Ok(SecurityVerifyResult {
            matches: false,
            plan,
        });
    }
}
