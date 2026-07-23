use anyhow::Result;
use clap::Args;

use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified::{Category, UnifiedOptions};

#[derive(Args)]
pub struct CommitCommand {
    #[command(subcommand)]
    action: CommitAction,
}

#[derive(clap::Subcommand)]
enum CommitAction {
    /// Preview managed file changes
    Plan,

    /// Commit managed files and create PRs
    Apply {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

impl CommitCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        let options = UnifiedOptions {
            categories: vec![Category::Files],
            allow_high_impact: false,
            verify: true,
        };
        match &self.action {
            CommitAction::Plan => crate::cli::plan::run_canonical_plan(
                client,
                manifest,
                options,
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "commit plan",
                    title: "Ward Commit Plan",
                },
            )
            .await
            .map(|_| ()),
            CommitAction::Apply { yes } => crate::cli::apply::run_canonical_apply(
                client,
                manifest,
                *yes,
                options,
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "commit apply",
                    title: "Ward Commit Apply",
                },
            )
            .await
            .map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn commit_plan_retains_its_subcommand() {
        let cli = crate::cli::Cli::parse_from(["ward", "commit", "plan", "--repo", "target"]);
        let crate::cli::Command::Commit(command) = cli.command else {
            panic!("expected commit command");
        };
        assert!(matches!(command.action, CommitAction::Plan));
    }

    #[test]
    fn commit_apply_retains_yes_flag() {
        let cli =
            crate::cli::Cli::parse_from(["ward", "commit", "apply", "--repo", "target", "--yes"]);
        let crate::cli::Command::Commit(command) = cli.command else {
            panic!("expected commit command");
        };
        assert!(matches!(command.action, CommitAction::Apply { yes: true }));
    }
}
