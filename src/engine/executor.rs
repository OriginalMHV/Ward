use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::github::Client;

use super::audit_log::AuditLog;
use super::planner::RepoPlan;

/// Execute a security plan, applying changes repo by repo.
pub async fn execute_security_plan(
    client: &Client,
    plans: &[RepoPlan],
    audit_log: &AuditLog,
) -> Result<ExecutionReport> {
    let multi = MultiProgress::new();
    let style = ProgressStyle::with_template(
        "  {spinner:.green} [{elapsed_precise}] {msg}",
    )?;

    let mut report = ExecutionReport::default();

    for plan in plans.iter().filter(|p| p.has_changes()) {
        let pb = multi.add(ProgressBar::new_spinner());
        pb.set_style(style.clone());
        pb.set_message(format!("{}: applying {} changes...", plan.repo, plan.changes.len()));

        match apply_repo_security(client, plan, audit_log).await {
            Ok(()) => {
                pb.finish_with_message(format!("{}: ✅ done", plan.repo));
                report.succeeded += 1;
            }
            Err(e) => {
                pb.finish_with_message(format!("{}: ❌ {e}", plan.repo));
                report.failed.push((plan.repo.clone(), e.to_string()));
            }
        }
    }

    Ok(report)
}

async fn apply_repo_security(
    client: &Client,
    plan: &RepoPlan,
    audit_log: &AuditLog,
) -> Result<()> {
    for change in &plan.changes {
        match change.feature.as_str() {
            "dependabot_alerts" if change.desired => {
                client.enable_dependabot_alerts(&plan.repo).await?;
            }
            "dependabot_security_updates" if change.desired => {
                client
                    .enable_dependabot_security_updates(&plan.repo)
                    .await?;
            }
            "secret_scanning" | "secret_scanning_ai_detection" | "push_protection" => {
                // These are set together via a single PATCH call
                // We'll collect them and apply once below
                continue;
            }
            _ => {
                tracing::warn!("Disabling {} is not yet supported", change.feature);
                continue;
            }
        }

        audit_log.log(
            &plan.repo,
            &format!("enable_{}", change.feature),
            "success",
            change.current,
            change.desired,
        )?;
    }

    // Apply secret scanning features if any changed
    let ss_changes: Vec<_> = plan
        .changes
        .iter()
        .filter(|c| {
            matches!(
                c.feature.as_str(),
                "secret_scanning" | "secret_scanning_ai_detection" | "push_protection"
            )
        })
        .collect();

    if !ss_changes.is_empty() {
        // Determine desired state for each (use desired from change, or keep current)
        let secret_scanning = ss_changes
            .iter()
            .find(|c| c.feature == "secret_scanning")
            .map(|c| c.desired)
            .unwrap_or(true);
        let ai_detection = ss_changes
            .iter()
            .find(|c| c.feature == "secret_scanning_ai_detection")
            .map(|c| c.desired)
            .unwrap_or(true);
        let push_protection = ss_changes
            .iter()
            .find(|c| c.feature == "push_protection")
            .map(|c| c.desired)
            .unwrap_or(true);

        client
            .set_security_features(&plan.repo, secret_scanning, ai_detection, push_protection)
            .await?;

        for change in &ss_changes {
            audit_log.log(
                &plan.repo,
                &format!("set_{}", change.feature),
                "success",
                change.current,
                change.desired,
            )?;
        }
    }

    Ok(())
}

#[derive(Debug, Default)]
pub struct ExecutionReport {
    pub succeeded: usize,
    pub failed: Vec<(String, String)>,
}

impl ExecutionReport {
    pub fn print_summary(&self) {
        use console::style;

        println!();
        if self.failed.is_empty() {
            println!(
                "  {} All {} repositories updated successfully.",
                style("✅").green(),
                self.succeeded
            );
        } else {
            println!(
                "  {} {} succeeded, {} {} failed:",
                style("⚠️").yellow(),
                self.succeeded,
                self.failed.len(),
                if self.failed.len() == 1 { "repo" } else { "repos" }
            );
            for (repo, err) in &self.failed {
                println!("    {} {}: {}", style("❌").red(), repo, err);
            }
        }
    }
}
