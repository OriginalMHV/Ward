use anyhow::Result;
use clap::Args;
use console::style;

use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified::{Category, UnifiedOptions};

#[derive(Args)]
pub struct ProtectionCommand {
    #[command(subcommand)]
    action: ProtectionAction,
}

#[derive(clap::Subcommand)]
enum ProtectionAction {
    /// Show what branch protection changes would be made (dry-run)
    Plan,

    /// Apply branch protection to default branches
    Apply {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Show current branch protection state
    Audit,
}

impl ProtectionCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        let options = UnifiedOptions {
            categories: vec![Category::BranchProtection],
            allow_high_impact: false,
            verify: true,
        };
        match &self.action {
            ProtectionAction::Plan => crate::cli::plan::run_canonical_plan(
                client,
                manifest,
                options,
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "protection plan",
                    title: "Ward Protection Plan",
                },
            )
            .await
            .map(|_| ()),
            ProtectionAction::Apply { yes } => crate::cli::apply::run_canonical_apply(
                client,
                manifest,
                *yes,
                options,
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "protection apply",
                    title: "Ward Protection Apply",
                },
            )
            .await
            .map(|_| ()),
            ProtectionAction::Audit => audit(client, manifest, system, repo).await,
        }
    }
}

async fn resolve_repos_with_branches(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<(String, String)>> {
    if let Some(repo_name) = repo {
        let repository = client.get_repo(repo_name).await?;
        return Ok(vec![(repository.name, repository.default_branch)]);
    }

    let system = system.ok_or_else(|| {
        anyhow::anyhow!("Either --system or --repo is required for protection commands")
    })?;
    let excludes = manifest.exclude_patterns_for_system(system);
    let explicit = manifest.explicit_repos_for_system(system);
    let repos = client
        .list_repos_for_system(
            system,
            manifest.matches_prefix_for_system(system),
            &excludes,
            &explicit,
        )
        .await?;
    Ok(repos
        .into_iter()
        .map(|repo| (repo.name, repo.default_branch))
        .collect())
}

async fn audit(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let repos = resolve_repos_with_branches(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Auditing branch protection for {} repositories...",
        style("[..]").bold(),
        repos.len()
    );

    use tabled::builder::Builder;
    use tabled::settings::object::{Columns, Rows};
    use tabled::settings::{Alignment, Modify, Style};

    let mut builder = Builder::default();
    builder.push_record([
        "Repository",
        "Branch",
        "PR Rev",
        "Approvals",
        "Stale",
        "Admins",
        "Linear",
        "Force",
    ]);

    let mut total_ok = 0;
    let mut total_issues = 0;

    for (repo_name, default_branch) in &repos {
        let state = client
            .get_branch_protection(repo_name, default_branch)
            .await?
            .unwrap_or_default();

        let protected = state.required_pull_request_reviews;
        if protected {
            total_ok += 1;
        } else {
            total_issues += 1;
        }

        let icon = |value: bool| {
            if value {
                format!("{}", style("[ok]").green())
            } else {
                format!("{}", style("[!!]").red())
            }
        };

        builder.push_record([
            repo_name.clone(),
            default_branch.clone(),
            icon(state.required_pull_request_reviews),
            state.required_approving_review_count.to_string(),
            icon(state.dismiss_stale_reviews),
            icon(state.enforce_admins),
            icon(state.required_linear_history),
            icon(state.allow_force_pushes),
        ]);
    }

    let table = builder
        .build()
        .with(Style::blank())
        .with(
            Modify::new(Rows::first()).with(tabled::settings::Format::content(|value| {
                format!("{}", style(value).bold().underlined())
            })),
        )
        .with(Modify::new(Columns::new(..)).with(Alignment::left()))
        .to_string();

    println!();
    for line in table.lines() {
        println!("  {line}");
    }

    println!();
    println!(
        "  Summary: {} protected, {} unprotected",
        style(total_ok).green().bold(),
        if total_issues > 0 {
            style(total_issues).red().bold()
        } else {
            style(total_issues).green().bold()
        }
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn protection_plan_retains_its_subcommand() {
        let cli = crate::cli::Cli::parse_from(["ward", "protection", "plan", "--repo", "target"]);
        let crate::cli::Command::Protection(command) = cli.command else {
            panic!("expected protection command");
        };
        assert!(matches!(command.action, ProtectionAction::Plan));
    }

    #[test]
    fn protection_apply_retains_yes_flag() {
        let cli = crate::cli::Cli::parse_from([
            "ward",
            "protection",
            "apply",
            "--repo",
            "target",
            "--yes",
        ]);
        let crate::cli::Command::Protection(command) = cli.command else {
            panic!("expected protection command");
        };
        assert!(matches!(
            command.action,
            ProtectionAction::Apply { yes: true }
        ));
    }
}
