use crate::outcome::Outcome;
use anyhow::Result;
use clap::Args;

use crate::cli::args::{CategoryArgs, OutputArgs, TargetArgs};
use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified::{self, UnifiedOptions, UnifiedReport};

/// Detect configuration drift from the desired manifest state.
#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DriftCommand {
    #[command(subcommand)]
    action: Option<DriftAction>,

    #[command(flatten)]
    args: DriftArgs,
}

#[derive(Args, Debug, Default)]
struct DriftArgs {
    #[command(flatten)]
    category: CategoryArgs,

    /// Include high-impact repository changes in the actionable drift count
    #[arg(long)]
    allow_high_impact: bool,

    #[command(flatten)]
    target: TargetArgs,

    #[command(flatten)]
    output: OutputArgs,
}

#[derive(clap::Subcommand)]
enum DriftAction {
    /// Deprecated alias of `ward drift`
    #[command(hide = true)]
    Check(DriftArgs),
}

impl DriftCommand {
    fn active_args(&self) -> &DriftArgs {
        match &self.action {
            Some(DriftAction::Check(args)) => args,
            None => &self.args,
        }
    }

    pub fn target(&self) -> &TargetArgs {
        &self.active_args().target
    }

    /// Print the deprecation warning when the run uses the `check` alias.
    pub fn announce(&self) {
        if self.action.is_some() {
            eprintln!("warning: 'ward drift check' is deprecated; use 'ward drift'");
        }
    }

    pub async fn run(&self, client: &Client, manifest: &Manifest) -> Result<()> {
        let args = self.active_args();
        run_drift(
            client,
            manifest,
            args.target.system.as_deref(),
            args.target.repo.as_deref(),
            args.output.is_json(),
            unified::select_categories(&args.category.categories),
            args.allow_high_impact,
        )
        .await
    }
}

/// Plan the selected categories and fail with a drift outcome when anything differs.
pub(crate) async fn run_drift(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    json: bool,
    categories: Vec<unified::Category>,
    allow_high_impact: bool,
) -> Result<()> {
    let options = UnifiedOptions {
        categories,
        allow_high_impact,
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
            command: "drift",
            title: "Ward Drift Check",
        },
    )
    .await?;
    fail_when_drifted(&report)
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

    fn parse(args: &[&str]) -> DriftCommand {
        let cli = crate::cli::Cli::parse_from(args);
        let crate::cli::Command::Drift(command) = cli.command else {
            panic!("expected drift command");
        };
        command
    }

    #[test]
    fn drift_defaults_to_all_categories() {
        let command = parse(&["ward", "drift", "--repo", "target"]);

        assert!(command.action.is_none());
        assert!(command.args.category.categories.is_empty());
        assert!(!command.args.allow_high_impact);
    }

    #[test]
    fn drift_accepts_category_filtering() {
        let command = parse(&["ward", "drift", "--category", "files,access", "--repo", "t"]);

        assert!(command.action.is_none());
        assert_eq!(
            command.args.category.categories,
            [unified::Category::Files, unified::Category::Access]
        );
    }

    #[test]
    fn deprecated_check_alias_still_parses_its_arguments() {
        let command = parse(&[
            "ward",
            "drift",
            "check",
            "--category",
            "files",
            "--allow-high-impact",
        ]);

        let Some(DriftAction::Check(args)) = command.action else {
            panic!("expected the check alias");
        };
        assert_eq!(args.category.categories, [unified::Category::Files]);
        assert!(args.allow_high_impact);
    }

    #[test]
    fn check_alias_is_hidden_from_help() {
        use clap::CommandFactory;
        let mut cli = crate::cli::Cli::command();
        let drift = cli.find_subcommand_mut("drift").unwrap();
        let help = drift.render_help().to_string();
        assert!(!help.contains("check"), "{help}");
    }

    #[test]
    fn drift_flags_and_the_check_subcommand_cannot_be_mixed() {
        let result =
            crate::cli::Cli::try_parse_from(["ward", "drift", "--category", "files", "check"]);
        assert!(result.is_err());
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
