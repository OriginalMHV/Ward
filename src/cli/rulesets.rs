use anyhow::Result;
use clap::Args;
use console::style;

use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified::{Category, UnifiedOptions};

#[derive(Args)]
pub struct RulesetsCommand {
    #[command(subcommand)]
    action: RulesetsAction,
}

#[derive(clap::Subcommand)]
enum RulesetsAction {
    /// Preview ruleset changes (dry-run)
    Plan,

    /// Apply rulesets to repositories
    Apply {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },

    /// Show current rulesets across repos
    Audit,
}

impl RulesetsCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        let options = UnifiedOptions {
            categories: vec![Category::Rulesets],
            allow_high_impact: false,
            verify: true,
        };
        match &self.action {
            RulesetsAction::Plan => crate::cli::plan::run_canonical_plan(
                client,
                manifest,
                options,
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "rulesets plan",
                    title: "Ward Rulesets Plan",
                },
            )
            .await
            .map(|_| ()),
            RulesetsAction::Apply { yes } => crate::cli::apply::run_canonical_apply(
                client,
                manifest,
                *yes,
                options,
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "rulesets apply",
                    title: "Ward Rulesets Apply",
                },
            )
            .await
            .map(|_| ()),
            RulesetsAction::Audit => audit(client, manifest, system, repo).await,
        }
    }
}

async fn resolve_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<String>> {
    if let Some(repo_name) = repo {
        return Ok(vec![repo_name.to_owned()]);
    }

    let sys = system.ok_or_else(|| {
        anyhow::anyhow!("Either --system or --repo is required for rulesets commands")
    })?;

    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(
            sys,
            manifest.matches_prefix_for_system(sys),
            &excludes,
            &explicit,
        )
        .await?;
    Ok(repos.into_iter().map(|r| r.name).collect())
}

async fn audit(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Auditing rulesets for {} repositories...",
        style("[..]").dim(),
        repos.len()
    );

    println!();
    println!(
        "  {} {}",
        style(format!("{:<40}", "Repository")).bold().underlined(),
        style("Rulesets").bold().underlined(),
    );
    println!("  {}", style("\u{2500}".repeat(70)).dim());

    for repo_name in &repos {
        let rulesets = client.list_rulesets(repo_name).await?;

        let summary = if rulesets.is_empty() {
            style("(none)").dim().to_string()
        } else {
            rulesets
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };

        println!("  {:<40} {}", repo_name, summary);
    }

    println!();
    println!(
        "  Summary: {} repositories scanned",
        style(repos.len()).green().bold()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn rulesets_plan_retains_its_subcommand() {
        let cli = crate::cli::Cli::parse_from(["ward", "rulesets", "plan", "--repo", "target"]);
        let crate::cli::Command::Rulesets(command) = cli.command else {
            panic!("expected rulesets command");
        };
        assert!(matches!(command.action, RulesetsAction::Plan));
    }

    #[test]
    fn rulesets_apply_retains_yes_flag() {
        let cli =
            crate::cli::Cli::parse_from(["ward", "rulesets", "apply", "--repo", "target", "--yes"]);
        let crate::cli::Command::Rulesets(command) = cli.command else {
            panic!("expected rulesets command");
        };
        assert!(matches!(
            command.action,
            RulesetsAction::Apply { yes: true }
        ));
    }
}
