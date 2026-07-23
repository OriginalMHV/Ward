use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::github::commits::CommitFile;

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
        match &self.action {
            CommitAction::Plan => plan_managed_files(client, manifest, system, repo).await,
            CommitAction::Apply { yes } => {
                crate::reconcile::unified::guard_legacy_mutation(
                    manifest,
                    crate::reconcile::unified::Category::Files,
                    "commit apply",
                )?;
                apply_managed_files(client, manifest, system, repo, *yes).await
            }
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
        let r = client.get_repo(repo_name).await?;
        return Ok(vec![(r.name, r.default_branch)]);
    }

    let sys = system.ok_or_else(|| anyhow::anyhow!("Either --system or --repo is required"))?;
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
    Ok(repos
        .into_iter()
        .map(|r| (r.name, r.default_branch))
        .collect())
}

struct ManagedFilePlan {
    repo_name: String,
    default_branch: String,
    files: Vec<CommitFile>,
}

async fn changed_managed_files(
    client: &Client,
    repo: &str,
    manifest: &Manifest,
) -> Result<Vec<CommitFile>> {
    let mut changed = Vec::new();

    for desired in &manifest.files {
        let matches = match client.get_file(repo, &desired.path, None).await? {
            Some(current) => Client::decode_content(&current)
                .map(|content| content == desired.content)
                .unwrap_or(false),
            None => false,
        };

        if !matches {
            changed.push(CommitFile {
                path: desired.path.clone(),
                content: desired.content.clone(),
            });
        }
    }

    Ok(changed)
}

pub(crate) async fn managed_files_compliant(
    client: &Client,
    repo: &str,
    manifest: &Manifest,
) -> Result<bool> {
    Ok(changed_managed_files(client, repo, manifest)
        .await?
        .is_empty())
}

async fn build_managed_file_plans(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<ManagedFilePlan>> {
    if manifest.files.is_empty() {
        anyhow::bail!(
            "No [[files]] entries configured. Run `ward init --from OWNER/REPO` to import managed files."
        );
    }

    let repos = resolve_repos_with_branches(client, manifest, system, repo).await?;
    let mut plans = Vec::with_capacity(repos.len());
    for (repo_name, default_branch) in repos {
        let files = changed_managed_files(client, &repo_name, manifest).await?;
        plans.push(ManagedFilePlan {
            repo_name,
            default_branch,
            files,
        });
    }
    Ok(plans)
}

async fn plan_managed_files(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let plans = build_managed_file_plans(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Managed file plan: {} configured file(s)",
        style("[..]").bold(),
        manifest.files.len()
    );
    println!();

    let mut changed_repos = 0usize;
    for plan in &plans {
        if plan.files.is_empty() {
            println!(
                "  {} {}",
                style("[ok]").green(),
                style(&plan.repo_name).dim()
            );
            continue;
        }

        changed_repos += 1;
        println!(
            "  {} {} ({} file(s))",
            style("[>>]").yellow(),
            style(&plan.repo_name).bold(),
            plan.files.len()
        );
        for file in &plan.files {
            println!("      {}", file.path);
        }
    }

    println!();
    println!(
        "  Summary: {} need changes, {} up to date",
        style(changed_repos).yellow().bold(),
        style(plans.len() - changed_repos).green()
    );

    if changed_repos > 0 {
        println!(
            "\n  Run {} to apply.",
            style("ward commit apply").cyan().bold()
        );
    }

    Ok(())
}

async fn apply_managed_files(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    yes: bool,
) -> Result<()> {
    let plans = build_managed_file_plans(client, manifest, system, repo).await?;
    let pending: Vec<&ManagedFilePlan> =
        plans.iter().filter(|plan| !plan.files.is_empty()).collect();

    if pending.is_empty() {
        println!(
            "\n  {} All managed files are already up to date.",
            style("[ok]").green()
        );
        return Ok(());
    }

    println!();
    println!(
        "  {} repos need managed file changes on branch {}:",
        style(pending.len()).yellow().bold(),
        style(&manifest.file_delivery.branch).cyan()
    );
    for plan in &pending {
        println!(
            "  {} {} - {} file(s)",
            style("[>>]").yellow(),
            plan.repo_name,
            plan.files.len()
        );
    }

    if !yes {
        println!();
        let proceed = Confirm::new()
            .with_prompt(format!(
                "  Commit managed files to {} repos and create PRs?",
                pending.len()
            ))
            .default(false)
            .interact()?;
        if !proceed {
            println!("  Aborted.");
            return Ok(());
        }
    }

    let audit_log = AuditLog::new()?;
    let mut succeeded = 0usize;
    let mut failed = Vec::new();

    for plan in pending {
        match commit_managed_files_and_pr(client, manifest, plan).await {
            Ok(pr_url) => {
                println!(
                    "  {} {}: {}",
                    style("[ok]").green(),
                    plan.repo_name,
                    style(pr_url).cyan()
                );
                audit_log.log(
                    &plan.repo_name,
                    "commit_managed_files",
                    "success",
                    false,
                    true,
                )?;
                succeeded += 1;
            }
            Err(error) => {
                println!("  {} {}: {error}", style("[!!]").red(), plan.repo_name);
                failed.push((plan.repo_name.clone(), error.to_string()));
            }
        }
    }

    println!();
    println!(
        "  Summary: {} succeeded, {} failed",
        style(succeeded).green().bold(),
        if failed.is_empty() {
            style(0).green().bold()
        } else {
            style(failed.len()).red().bold()
        }
    );
    for (repo, error) in &failed {
        println!("    {} {repo}: {error}", style("[!!]").red());
    }

    if !failed.is_empty() {
        anyhow::bail!(
            "{} of {} repositories failed during commit apply",
            failed.len(),
            succeeded + failed.len()
        );
    }

    Ok(())
}

async fn commit_managed_files_and_pr(
    client: &Client,
    manifest: &Manifest,
    plan: &ManagedFilePlan,
) -> Result<String> {
    let branch = &manifest.file_delivery.branch;
    let prefix = &manifest.file_delivery.commit_message_prefix;
    client
        .ensure_dedicated_branch(&plan.repo_name, branch, &plan.default_branch)
        .await?;
    client
        .create_commit(
            &plan.repo_name,
            branch,
            &format!("{prefix}sync repository configuration"),
            &plan.files,
        )
        .await?;

    let paths = plan
        .files
        .iter()
        .map(|file| format!("- `{}`", file.path))
        .collect::<Vec<_>>()
        .join("\n");
    let body = format!(
        "## Ward: sync repository configuration\n\n\
         Imported managed files:\n\n{paths}\n\n\
         This PR was created by [Ward](https://github.com/OriginalMHV/Ward)."
    );
    let pull = client
        .create_pull_request(
            &plan.repo_name,
            &format!("{prefix}sync repository configuration"),
            &body,
            branch,
            &plan.default_branch,
            &manifest.file_delivery.reviewers,
        )
        .await?;

    Ok(pull.html_url)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn commit_plan_selects_managed_files() {
        let cli = crate::cli::Cli::parse_from(["ward", "commit", "plan", "--repo", "target"]);
        let crate::cli::Command::Commit(command) = cli.command else {
            panic!("expected commit command");
        };
        assert!(matches!(command.action, CommitAction::Plan));
    }

    #[test]
    fn commit_apply_defaults_to_interactive() {
        let cli = crate::cli::Cli::parse_from(["ward", "commit", "apply", "--repo", "target"]);
        let crate::cli::Command::Commit(command) = cli.command else {
            panic!("expected commit command");
        };
        assert!(matches!(command.action, CommitAction::Apply { yes: false }));
    }
}
