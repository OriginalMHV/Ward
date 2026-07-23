use anyhow::Result;
use clap::Args;

use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified::{self, UnifiedOptions, UnifiedReport};

#[derive(Args)]
pub struct DriftCommand {
    #[command(subcommand)]
    action: DriftAction,
}

#[derive(clap::Subcommand)]
enum DriftAction {
    /// Check for configuration drift across repos
    Check {
        /// Limit to one or more categories (repeatable). Defaults to all categories.
        #[arg(long = "category", value_name = "CATEGORY")]
        categories: Vec<String>,

        /// Include high-impact repository changes in the actionable drift count
        #[arg(long)]
        allow_high_impact: bool,
    },
}

impl DriftCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
        json: bool,
    ) -> Result<()> {
        match &self.action {
            DriftAction::Check {
                categories,
                allow_high_impact,
            } => {
                let options = UnifiedOptions {
                    categories: unified::parse_categories(categories)?,
                    allow_high_impact: *allow_high_impact,
                    verify: true,
                };
                let report = crate::cli::plan::run_canonical_plan(
                    client,
                    manifest,
                    options,
                    crate::cli::plan::CategoryRun {
                        system,
                        repo,
                        json,
                        command: "drift check",
                        title: "Ward Drift Check",
                    },
                )
                .await?;
                fail_when_drifted(&report)
            }
        }
    }
}

fn fail_when_drifted(report: &UnifiedReport) -> Result<()> {
    if report.actionable == 0 && !report.has_failures() {
        return Ok(());
    }

    anyhow::bail!(
        "Drift check found {} actionable change(s) and {} blocked category result(s); see report above",
        report.actionable,
        report.blocked
    );
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn drift_check_defaults_to_all_categories() {
        let cli = crate::cli::Cli::parse_from(["ward", "drift", "check", "--repo", "target"]);
        let crate::cli::Command::Drift(command) = cli.command else {
            panic!("expected drift command");
        };

        assert!(matches!(
            command.action,
            DriftAction::Check {
                categories,
                allow_high_impact: false,
            } if categories.is_empty()
        ));
    }

    #[test]
    fn drift_check_accepts_category_filtering() {
        let cli = crate::cli::Cli::parse_from([
            "ward",
            "drift",
            "check",
            "--category",
            "files",
            "--repo",
            "target",
        ]);
        let crate::cli::Command::Drift(command) = cli.command else {
            panic!("expected drift command");
        };

        assert!(matches!(
            command.action,
            DriftAction::Check { categories, .. } if categories == ["files"]
        ));
    }

    #[test]
    fn drift_exit_is_zero_without_actionable_or_blocked_results() {
        let report = UnifiedReport::from_repos(Vec::new());
        assert!(fail_when_drifted(&report).is_ok());
    }

    #[test]
    fn drift_exit_is_non_zero_for_actionable_results() {
        let mut report = UnifiedReport::from_repos(Vec::new());
        report.actionable = 1;
        assert!(fail_when_drifted(&report).is_err());
    }

    #[test]
    fn drift_exit_is_non_zero_for_blocked_results() {
        let mut report = UnifiedReport::from_repos(Vec::new());
        report.blocked = 1;
        assert!(fail_when_drifted(&report).is_err());
    }
}
