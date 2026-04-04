use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::github::branch_protection::BranchProtectionState;

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
        match &self.action {
            ProtectionAction::Plan => plan(client, manifest, system, repo).await,
            ProtectionAction::Apply { yes } => apply(client, manifest, system, repo, *yes).await,
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
        let r = client.get_repo(repo_name).await?;
        return Ok(vec![(r.name, r.default_branch)]);
    }

    let sys = system.ok_or_else(|| {
        anyhow::anyhow!("Either --system or --repo is required for protection commands")
    })?;

    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(sys, &excludes, &explicit)
        .await?;
    Ok(repos
        .into_iter()
        .map(|r| (r.name, r.default_branch))
        .collect())
}

struct ProtectionDiff {
    repo: String,
    branch: String,
    changes: Vec<ProtectionChange>,
}

struct ProtectionChange {
    field: String,
    current: String,
    desired: String,
}

impl ProtectionDiff {
    fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }
}

fn diff_protection(
    repo: &str,
    branch: &str,
    current: &BranchProtectionState,
    config: &crate::config::manifest::BranchProtectionConfig,
) -> ProtectionDiff {
    let mut changes = Vec::new();

    let checks: Vec<(&str, String, String)> = vec![
        (
            "required_pull_request_reviews",
            current.required_pull_request_reviews.to_string(),
            config.enabled.to_string(),
        ),
        (
            "required_approvals",
            current.required_approving_review_count.to_string(),
            config.required_approvals.to_string(),
        ),
        (
            "dismiss_stale_reviews",
            current.dismiss_stale_reviews.to_string(),
            config.dismiss_stale_reviews.to_string(),
        ),
        (
            "require_code_owner_reviews",
            current.require_code_owner_reviews.to_string(),
            config.require_code_owner_reviews.to_string(),
        ),
        (
            "require_status_checks",
            current.required_status_checks.to_string(),
            config.require_status_checks.to_string(),
        ),
        (
            "strict_status_checks",
            current.strict_status_checks.to_string(),
            config.strict_status_checks.to_string(),
        ),
        (
            "enforce_admins",
            current.enforce_admins.to_string(),
            config.enforce_admins.to_string(),
        ),
        (
            "required_linear_history",
            current.required_linear_history.to_string(),
            config.required_linear_history.to_string(),
        ),
        (
            "allow_force_pushes",
            current.allow_force_pushes.to_string(),
            config.allow_force_pushes.to_string(),
        ),
        (
            "allow_deletions",
            current.allow_deletions.to_string(),
            config.allow_deletions.to_string(),
        ),
    ];

    for (field, current_val, desired_val) in checks {
        if current_val != desired_val {
            changes.push(ProtectionChange {
                field: field.to_string(),
                current: current_val,
                desired: desired_val,
            });
        }
    }

    ProtectionDiff {
        repo: repo.to_string(),
        branch: branch.to_string(),
        changes,
    }
}

async fn build_diffs(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<ProtectionDiff>> {
    let repos = resolve_repos_with_branches(client, manifest, system, repo).await?;
    let config = &manifest.branch_protection;

    println!();
    println!(
        "  {} Scanning {} repositories...",
        style("[..]").bold(),
        repos.len()
    );

    let mut diffs = Vec::new();
    for (repo_name, default_branch) in &repos {
        let current = client
            .get_branch_protection(repo_name, default_branch)
            .await?
            .unwrap_or_default();

        diffs.push(diff_protection(repo_name, default_branch, &current, config));
    }

    Ok(diffs)
}

async fn plan(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let diffs = build_diffs(client, manifest, system, repo).await?;

    print_diff_table(&diffs);

    let needs_changes = diffs.iter().filter(|d| d.has_changes()).count();
    if needs_changes > 0 {
        println!(
            "\n  Run {} to apply these changes.",
            style("ward protection apply").cyan().bold()
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
) -> Result<()> {
    let diffs = build_diffs(client, manifest, system, repo).await?;

    let needs_changes = diffs.iter().filter(|d| d.has_changes()).count();
    if needs_changes == 0 {
        println!(
            "\n  {} All repositories are up to date.",
            style("[ok]").green()
        );
        return Ok(());
    }

    print_diff_table(&diffs);

    if !yes {
        let proceed = Confirm::new()
            .with_prompt(format!(
                "  Apply branch protection to {needs_changes} repositories?"
            ))
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
    let config = &manifest.branch_protection;
    let mut succeeded = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    for diff in diffs.iter().filter(|d| d.has_changes()) {
        match client
            .update_branch_protection(&diff.repo, &diff.branch, config)
            .await
        {
            Ok(()) => {
                println!(
                    "  {} {}/{}: done",
                    style(">>").magenta(),
                    diff.repo,
                    diff.branch
                );
                audit_log.log(
                    &diff.repo,
                    "update_branch_protection",
                    "success",
                    false,
                    true,
                )?;
                succeeded += 1;
            }
            Err(e) => {
                println!(
                    "  {} {}/{}: error: {e}",
                    style(">>").magenta(),
                    diff.repo,
                    diff.branch
                );
                failed.push((diff.repo.clone(), e.to_string()));
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!(
            "  {} All {} repositories updated successfully.",
            style("[ok]").green(),
            succeeded
        );
    } else {
        println!(
            "  {} {} succeeded, {} failed:",
            style("[warn]").yellow(),
            succeeded,
            failed.len()
        );
        for (repo, err) in &failed {
            println!("    {} {}: {}", style("[!!]").red(), repo, err);
        }
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
    let repos = resolve_repos_with_branches(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Auditing branch protection for {} repositories...",
        style("[..]").bold(),
        repos.len()
    );

    println!();
    println!(
        "  {:40} {:8} {:10} {:10} {:10} {:10} {:10} {:10}",
        style("Repository").bold().underlined(),
        style("Branch").bold().underlined(),
        style("PR Rev").bold().underlined(),
        style("Approvals").bold().underlined(),
        style("Stale").bold().underlined(),
        style("Admins").bold().underlined(),
        style("Linear").bold().underlined(),
        style("Force").bold().underlined(),
    );

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

        let icon = |v: bool| {
            if v {
                format!("{}", style("[ok]").green())
            } else {
                format!("{}", style("[!!]").red())
            }
        };

        println!(
            "  {:40} {:8} {:10} {:10} {:10} {:10} {:10} {:10}",
            repo_name,
            default_branch,
            icon(state.required_pull_request_reviews),
            state.required_approving_review_count,
            icon(state.dismiss_stale_reviews),
            icon(state.enforce_admins),
            icon(state.required_linear_history),
            icon(state.allow_force_pushes),
        );
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

fn print_diff_table(diffs: &[ProtectionDiff]) {
    println!();
    println!("  {}", style("Branch Protection Plan").bold().cyan());
    println!("  {}", style("─".repeat(60)).dim());

    for diff in diffs {
        if diff.has_changes() {
            println!(
                "  {} {} ({})",
                style("[>>]").yellow(),
                style(&diff.repo).bold(),
                diff.branch
            );
            for change in &diff.changes {
                println!(
                    "     {}: {} -> {}",
                    change.field,
                    style(&change.current).red(),
                    style(&change.desired).green().bold()
                );
            }
        } else {
            println!("  {} {}", style("[ok]").green(), style(&diff.repo).dim());
        }
    }

    let needs_changes = diffs.iter().filter(|d| d.has_changes()).count();
    let up_to_date = diffs.len() - needs_changes;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::BranchProtectionConfig;

    fn default_state() -> BranchProtectionState {
        BranchProtectionState {
            required_pull_request_reviews: false,
            required_approving_review_count: 1,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            required_status_checks: false,
            strict_status_checks: false,
            enforce_admins: false,
            required_linear_history: false,
            allow_force_pushes: false,
            allow_deletions: false,
        }
    }

    fn default_config() -> BranchProtectionConfig {
        BranchProtectionConfig {
            enabled: false,
            required_approvals: 1,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            require_status_checks: false,
            strict_status_checks: false,
            enforce_admins: false,
            required_linear_history: false,
            allow_force_pushes: false,
            allow_deletions: false,
        }
    }

    #[test]
    fn no_changes_when_state_matches_config() {
        let state = default_state();
        let config = default_config();
        let diff = diff_protection("my-repo", "main", &state, &config);
        assert!(!diff.has_changes());
    }

    #[test]
    fn all_fields_produce_changes_when_they_differ() {
        let state = default_state();
        let config = BranchProtectionConfig {
            enabled: true,
            required_approvals: 2,
            dismiss_stale_reviews: true,
            require_code_owner_reviews: true,
            require_status_checks: true,
            strict_status_checks: true,
            enforce_admins: true,
            required_linear_history: true,
            allow_force_pushes: true,
            allow_deletions: true,
        };
        let diff = diff_protection("my-repo", "main", &state, &config);
        assert_eq!(diff.changes.len(), 10);
    }

    #[test]
    fn partial_changes_detected() {
        let state = default_state();
        let mut config = default_config();
        config.enforce_admins = true;
        config.required_approvals = 3;

        let diff = diff_protection("my-repo", "main", &state, &config);
        assert_eq!(diff.changes.len(), 2);
        let fields: Vec<&str> = diff.changes.iter().map(|c| c.field.as_str()).collect();
        assert!(fields.contains(&"enforce_admins"));
        assert!(fields.contains(&"required_approvals"));
    }

    #[test]
    fn repo_and_branch_preserved() {
        let state = default_state();
        let config = default_config();
        let diff = diff_protection("acme-service", "develop", &state, &config);
        assert_eq!(diff.repo, "acme-service");
        assert_eq!(diff.branch, "develop");
    }
}
