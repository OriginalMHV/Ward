use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::engine::{audit_log::AuditLog, executor, planner, verifier};
use crate::github::Client;

#[derive(Args)]
pub struct SecurityCommand {
    #[command(subcommand)]
    action: SecurityAction,
}

#[derive(clap::Subcommand)]
enum SecurityAction {
    /// Show what security changes would be made (dry-run)
    Plan,

    /// Apply security changes to repositories
    Apply {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,

        /// Skip post-apply verification
        #[arg(long)]
        skip_verify: bool,
    },

    /// Audit current security state across all repos
    Audit,
}

impl SecurityCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        match &self.action {
            SecurityAction::Plan => plan(client, manifest, system, repo).await,
            SecurityAction::Apply { yes, skip_verify } => {
                apply(client, manifest, system, repo, *yes, *skip_verify).await
            }
            SecurityAction::Audit => audit(client, manifest, system, repo).await,
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
        anyhow::anyhow!("Either --system or --repo is required for security commands")
    })?;

    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(sys, &excludes, &explicit)
        .await?;
    Ok(repos.into_iter().map(|r| r.name).collect())
}

async fn build_plans(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<(Vec<planner::RepoPlan>, String)> {
    let repo_names = resolve_repos(client, manifest, system, repo).await?;
    let sys_id = system.unwrap_or("default");
    let desired = manifest.security_for_system(sys_id);

    println!();
    println!(
        "  {} Scanning {} repositories...",
        style("[..]").bold(),
        repo_names.len()
    );

    let mut plans = Vec::new();
    for repo_name in &repo_names {
        let current = client.get_security_state(repo_name).await?;
        let plan = planner::plan_security(repo_name, &current, desired);
        plans.push(plan);
    }

    Ok((plans, sys_id.to_owned()))
}

async fn plan(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let (plans, sys_id) = build_plans(client, manifest, system, repo).await?;

    print_plan_table(&plans, &sys_id);

    let needs_changes = plans.iter().filter(|p| p.has_changes()).count();
    if needs_changes > 0 {
        println!(
            "\n  Run {} to apply these changes.",
            style("ward security apply").cyan().bold()
        );
    }

    Ok(())
}

async fn apply(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    yes: bool,
    skip_verify: bool,
) -> Result<()> {
    let (plans, sys_id) = build_plans(client, manifest, system, repo).await?;

    let needs_changes = plans.iter().filter(|p| p.has_changes()).count();
    if needs_changes == 0 {
        println!(
            "\n  {} All repositories are up to date.",
            style("[ok]").green()
        );
        return Ok(());
    }

    print_plan_table(&plans, &sys_id);

    if !yes {
        let proceed = Confirm::new()
            .with_prompt(format!("  Apply changes to {needs_changes} repositories?"))
            .default(false)
            .interact()?;

        if !proceed {
            println!("  Aborted.");
            return Ok(());
        }
    }

    println!();
    println!("  {} Applying changes...", style("[>>]").bold());

    let audit_log = AuditLog::new()?;
    let report = executor::execute_security_plan(client, &plans, &audit_log).await?;
    report.print_summary();

    if !skip_verify && report.failed.is_empty() {
        println!();
        println!("  {} Verifying changes...", style("[..]").bold());

        let desired = manifest.security_for_system(&sys_id);
        let verify_report = verifier::verify_security(client, &plans, desired).await?;
        verify_report.print_summary();
    }

    println!(
        "\n  {} Audit log: {}",
        style("[..]").bold(),
        audit_log.path().display()
    );

    Ok(())
}

async fn audit(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let repo_names = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Auditing {} repositories...",
        style("[..]").bold(),
        repo_names.len()
    );

    println!();
    println!(
        "  {:40} {:8} {:8} {:8} {:8} {:8}",
        style("Repository").bold().underlined(),
        style("Dep.A").bold().underlined(),
        style("Dep.SU").bold().underlined(),
        style("Secret").bold().underlined(),
        style("AI").bold().underlined(),
        style("Push").bold().underlined(),
    );

    let mut total_ok = 0;
    let mut total_issues = 0;

    for repo_name in &repo_names {
        let state = client.get_security_state(repo_name).await?;

        let features = [
            state.dependabot_alerts,
            state.dependabot_security_updates,
            state.secret_scanning,
            state.secret_scanning_ai_detection,
            state.push_protection,
        ];

        let all_ok = features.iter().all(|&f| f);
        if all_ok {
            total_ok += 1;
        } else {
            total_issues += 1;
        }

        let icons: Vec<String> = features
            .iter()
            .map(|&f| {
                if f {
                    format!("{}", style("[ok]").green())
                } else {
                    format!("{}", style("[!!]").red())
                }
            })
            .collect();

        println!(
            "  {:40} {:8} {:8} {:8} {:8} {:8}",
            repo_name, icons[0], icons[1], icons[2], icons[3], icons[4]
        );
    }

    println!();
    println!(
        "  Summary: {} fully secured, {} need attention",
        style(total_ok).green().bold(),
        if total_issues > 0 {
            style(total_issues).red().bold()
        } else {
            style(total_issues).green().bold()
        }
    );

    Ok(())
}

fn print_plan_table(plans: &[planner::RepoPlan], system_id: &str) {
    println!();
    println!(
        "  {}",
        style(format!("Security Plan: {system_id}")).bold().cyan()
    );
    println!("  {}", style("─".repeat(60)).dim());

    for plan in plans {
        if plan.has_changes() {
            println!("  {} {}", style("[>>]").yellow(), style(&plan.repo).bold());
            for change in &plan.changes {
                let current = if change.current {
                    style("on").green()
                } else {
                    style("off").red()
                };
                let desired = if change.desired {
                    style("on").green().bold()
                } else {
                    style("off").red().bold()
                };
                println!("     {}: {current} -> {desired}", change.feature);
            }
        } else {
            println!("  {} {}", style("[ok]").green(), style(&plan.repo).dim());
        }
    }

    let needs_changes = plans.iter().filter(|p| p.has_changes()).count();
    let up_to_date = plans.len() - needs_changes;

    println!();
    println!(
        "  Summary: {} need changes, {} up to date",
        if needs_changes > 0 {
            style(needs_changes).yellow().bold()
        } else {
            style(needs_changes).green().bold()
        },
        style(up_to_date).green()
    );
}
