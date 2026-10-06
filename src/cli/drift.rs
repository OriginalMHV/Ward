use crate::outcome::Outcome;
use anyhow::Result;
use clap::Args;

use crate::cli::args::CategoryArgs;
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
        #[command(flatten)]
        category: CategoryArgs,

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
                category,
                allow_high_impact,
            } => {
                let options = UnifiedOptions {
                    categories: unified::select_categories(&category.categories),
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
    let unknown = report.has_unknown_managed_state();
    if report.actionable == 0 && report.deferred == 0 && !unknown && !report.has_failures() {
        return Ok(());
    }

    let (failed, blocked) = report.category_problem_counts();
    Err(Outcome::Drift(crate::outcome::drift_summary(
        report.actionable,
        report.deferred,
        failed,
        blocked,
        unknown,
    ))
    .into())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn drift_is_an_outcome_and_a_clean_report_is_not() {
        let clean = UnifiedReport::from_repos(Vec::new());
        assert!(fail_when_drifted(&clean).is_ok());

        let mut drifted = UnifiedReport::from_repos(Vec::new());
        drifted.actionable = 1;
        let error = fail_when_drifted(&drifted).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Outcome>(),
            Some(Outcome::Drift(_))
        ));
    }

    #[test]
    fn drift_check_defaults_to_all_categories() {
        let cli = crate::cli::Cli::parse_from(["ward", "drift", "check", "--repo", "target"]);
        let crate::cli::Command::Drift(command) = cli.command else {
            panic!("expected drift command");
        };

        assert!(matches!(
            command.action,
            DriftAction::Check {
                category,
                allow_high_impact: false,
            } if category.categories.is_empty()
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
            DriftAction::Check { category, .. } if category.categories == [unified::Category::Files]
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

    fn category_report(disposition: &str, outcome: Option<&str>) -> unified::CategoryReport {
        unified::CategoryReport {
            category: "rulesets".to_owned(),
            disposition: disposition.to_owned(),
            status: "noop".to_owned(),
            actionable: 0,
            blocked: 0,
            warnings: 0,
            deferred: 0,
            coverage: Default::default(),
            coverage_outcomes: outcome
                .map(|outcome| unified::CoverageOutcomeCount {
                    outcome: outcome.to_owned(),
                    count: 1,
                })
                .into_iter()
                .collect(),
            details: Vec::new(),
            error: None,
            verified: None,
            configuration_pull_request_pending: false,
        }
    }

    fn report_with(category: unified::CategoryReport) -> UnifiedReport {
        UnifiedReport::from_repos(vec![unified::RepoReport {
            repo: "repo".to_owned(),
            actionable: category.actionable,
            blocked: category.blocked,
            warnings: category.warnings,
            deferred: category.deferred,
            categories: vec![category],
        }])
    }

    #[test]
    fn drift_exit_is_non_zero_for_deferred_changes() {
        let mut category = category_report("managed", None);
        category.deferred = 2;
        assert!(fail_when_drifted(&report_with(category)).is_err());
    }

    #[test]
    fn drift_exit_is_non_zero_for_unreadable_managed_state() {
        let category = category_report("managed", Some("permission_denied"));
        assert!(fail_when_drifted(&report_with(category)).is_err());
    }

    #[test]
    fn drift_exit_ignores_unreadable_observe_state() {
        let category = category_report("observe", Some("permission_denied"));
        assert!(fail_when_drifted(&report_with(category)).is_ok());
    }
}
