use anyhow::Result;
use clap::Args;
use console::style;

use crate::engine::audit_log::{self, AuditEntry};
use crate::github::Client;

#[derive(Args)]
pub struct RollbackCommand {
    /// Show last N audit entries
    #[arg(long, default_value_t = 10)]
    last: usize,

    /// Filter to a specific repository
    #[arg(long)]
    repo: Option<String>,

    /// Show what would be reversed without applying
    #[arg(long)]
    dry_run: bool,

    /// Skip confirmation prompt
    #[arg(long)]
    yes: bool,
}

impl RollbackCommand {
    pub async fn run(&self, client: &Client) -> Result<()> {
        let log_path = audit_log::default_log_path()?;

        if !log_path.exists() {
            println!(
                "\n  {} No audit log found at {}",
                style("⚠️").yellow(),
                log_path.display()
            );
            return Ok(());
        }

        let entries = audit_log::read_entries(&log_path)?;
        let successful: Vec<&AuditEntry> = entries
            .iter()
            .filter(|e| e.status == "success")
            .filter(|e| self.repo.as_ref().is_none_or(|r| e.repo == *r))
            .collect();

        let to_process: Vec<&AuditEntry> =
            successful.iter().rev().take(self.last).copied().collect();

        if to_process.is_empty() {
            println!(
                "\n  {} No matching audit entries found.",
                style("ℹ️").blue()
            );
            return Ok(());
        }

        println!();
        println!(
            "  {} Rollback candidates ({} entries):",
            style("🔄").bold(),
            to_process.len()
        );
        println!();

        let mut reversible = Vec::new();
        let mut skipped = Vec::new();

        for entry in &to_process {
            match classify_rollback(entry) {
                RollbackAction::Reverse(desc) => {
                    println!(
                        "  {} {} / {} / {}",
                        style("⚡").yellow(),
                        entry.repo,
                        entry.action,
                        desc
                    );
                    reversible.push((*entry, desc));
                }
                RollbackAction::Skip(reason) => {
                    println!(
                        "  {} {} / {} / {}",
                        style("⏭").dim(),
                        style(&entry.repo).dim(),
                        style(&entry.action).dim(),
                        style(&reason).dim()
                    );
                    skipped.push((*entry, reason));
                }
            }
        }

        println!();
        println!(
            "  {} reversible, {} skipped",
            style(reversible.len()).yellow().bold(),
            style(skipped.len()).dim()
        );

        if reversible.is_empty() {
            println!("\n  {} Nothing to rollback.", style("ℹ️").blue());
            return Ok(());
        }

        if self.dry_run {
            println!("\n  {} Dry run - no changes applied.", style("ℹ️").blue());
            return Ok(());
        }

        if !self.yes {
            let proceed = dialoguer::Confirm::new()
                .with_prompt(format!("  Rollback {} entries?", reversible.len()))
                .default(false)
                .interact()?;

            if !proceed {
                println!("  Aborted.");
                return Ok(());
            }
        }

        println!();
        println!("  {} Rolling back...", style("⚡").bold());

        let mut succeeded = 0usize;
        let mut failed: Vec<(String, String)> = Vec::new();

        for (entry, _desc) in &reversible {
            match execute_rollback(client, entry).await {
                Ok(()) => {
                    println!(
                        "  {} {}/{}: ✅ rolled back",
                        style("▶").magenta(),
                        entry.repo,
                        entry.action
                    );
                    succeeded += 1;
                }
                Err(e) => {
                    println!(
                        "  {} {}/{}: ❌ {e}",
                        style("▶").magenta(),
                        entry.repo,
                        entry.action
                    );
                    failed.push((entry.repo.clone(), e.to_string()));
                }
            }
        }

        println!();
        if failed.is_empty() {
            println!(
                "  {} All {} entries rolled back successfully.",
                style("✅").green(),
                succeeded
            );
        } else {
            println!(
                "  {} {} succeeded, {} failed:",
                style("⚠️").yellow(),
                succeeded,
                failed.len()
            );
            for (repo, err) in &failed {
                println!("    {} {}: {}", style("❌").red(), repo, err);
            }
        }

        Ok(())
    }
}

enum RollbackAction {
    Reverse(String),
    Skip(String),
}

fn classify_rollback(entry: &AuditEntry) -> RollbackAction {
    match entry.action.as_str() {
        "set_secret_scanning" if entry.after == serde_json::Value::Bool(true) => {
            RollbackAction::Reverse("disable secret scanning".to_string())
        }
        "set_push_protection" if entry.after == serde_json::Value::Bool(true) => {
            RollbackAction::Reverse("disable push protection".to_string())
        }
        "set_secret_scanning_ai_detection" if entry.after == serde_json::Value::Bool(true) => {
            RollbackAction::Reverse("disable secret scanning AI detection".to_string())
        }
        "enable_dependabot_alerts" => {
            RollbackAction::Skip("disabling Dependabot alerts not supported via API".to_string())
        }
        "enable_dependabot_security_updates" => RollbackAction::Skip(
            "disabling Dependabot security updates not supported via API".to_string(),
        ),
        "create_copilot_review_ruleset" => RollbackAction::Skip(
            "ruleset deletion requires ruleset ID - manual removal needed".to_string(),
        ),
        "deploy_copilot_instructions" => {
            RollbackAction::Skip("file deletion not supported - remove via PR".to_string())
        }
        "update_branch_protection" => RollbackAction::Skip(
            "branch protection rollback not supported - re-run with desired config".to_string(),
        ),
        _ => RollbackAction::Skip(format!("unknown action: {}", entry.action)),
    }
}

async fn execute_rollback(client: &Client, entry: &AuditEntry) -> Result<()> {
    match entry.action.as_str() {
        "set_secret_scanning" => {
            client
                .set_security_features(&entry.repo, false, true, true)
                .await
        }
        "set_push_protection" => {
            client
                .set_security_features(&entry.repo, true, true, false)
                .await
        }
        "set_secret_scanning_ai_detection" => {
            client
                .set_security_features(&entry.repo, true, false, true)
                .await
        }
        _ => anyhow::bail!("Cannot rollback action: {}", entry.action),
    }
}
