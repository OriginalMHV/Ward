use anyhow::Result;

use crate::config::SecurityConfig;
use crate::github::Client;

use super::planner::{self, RepoPlan};

/// Re-read security state and verify it matches the plan.
pub async fn verify_security(
    client: &Client,
    plans: &[RepoPlan],
    desired: &SecurityConfig,
) -> Result<VerificationReport> {
    let mut report = VerificationReport::default();

    for plan in plans.iter().filter(|p| p.has_changes()) {
        let current = client.get_security_state(&plan.repo).await?;
        let re_plan = planner::plan_security(&plan.repo, &current, desired);

        if re_plan.has_changes() {
            report.mismatches.push(VerificationMismatch {
                repo: plan.repo.clone(),
                remaining_changes: re_plan
                    .changes
                    .iter()
                    .map(|c| format!("{}: {} (expected {})", c.feature, c.current, c.desired))
                    .collect(),
            });
        } else {
            report.verified += 1;
        }
    }

    Ok(report)
}

#[derive(Debug, Default)]
pub struct VerificationReport {
    pub verified: usize,
    pub mismatches: Vec<VerificationMismatch>,
}

#[derive(Debug)]
pub struct VerificationMismatch {
    pub repo: String,
    pub remaining_changes: Vec<String>,
}

impl VerificationReport {
    pub fn print_summary(&self) {
        use console::style;

        println!();
        if self.mismatches.is_empty() {
            println!(
                "  {} Verification passed: all {} repos match desired state.",
                style("✅").green(),
                self.verified
            );
        } else {
            println!(
                "  {} Verification: {} passed, {} mismatches:",
                style("⚠️").yellow(),
                self.verified,
                self.mismatches.len()
            );
            for m in &self.mismatches {
                println!("    {} {}:", style("❌").red(), m.repo);
                for change in &m.remaining_changes {
                    println!("      - {change}");
                }
            }
        }
    }
}
