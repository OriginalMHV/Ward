use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::config::manifest::TeamAccess;
use crate::github::Client;
use crate::github::teams::Team;

#[derive(Args)]
pub struct TeamsCommand {
    #[command(subcommand)]
    action: TeamsAction,
}

#[derive(clap::Subcommand)]
enum TeamsAction {
    /// List teams and their repo access
    List,

    /// Preview team access changes (dry-run)
    Plan,

    /// Apply team access to repositories
    Apply {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },

    /// Audit current team access across repos
    Audit,
}

impl TeamsCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        match &self.action {
            TeamsAction::List => list(client, manifest, system, repo).await,
            TeamsAction::Plan => plan(client, manifest, system, repo).await,
            TeamsAction::Apply { yes } => apply(client, manifest, system, repo, *yes).await,
            TeamsAction::Audit => audit(client, manifest, system, repo).await,
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
        anyhow::anyhow!("Either --system or --repo is required for teams commands")
    })?;

    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(sys, &excludes, &explicit)
        .await?;
    Ok(repos.into_iter().map(|r| r.name).collect())
}

fn teams_for_system<'a>(manifest: &'a Manifest, system_id: &str) -> &'a [TeamAccess] {
    manifest
        .system(system_id)
        .map(|s| s.teams.as_slice())
        .unwrap_or(&[])
}

struct TeamDiff {
    repo: String,
    to_add: Vec<TeamAccess>,
    to_update: Vec<TeamAccess>,
    to_remove: Vec<String>,
}

impl TeamDiff {
    fn has_changes(&self) -> bool {
        !self.to_add.is_empty() || !self.to_update.is_empty() || !self.to_remove.is_empty()
    }

    fn change_count(&self) -> usize {
        self.to_add.len() + self.to_update.len() + self.to_remove.len()
    }
}

fn diff_teams(repo: &str, desired: &[TeamAccess], current: &[Team]) -> TeamDiff {
    let mut to_add = Vec::new();
    let mut to_update = Vec::new();
    let mut to_remove = Vec::new();

    for d in desired {
        match current.iter().find(|c| c.slug == d.slug) {
            None => to_add.push(d.clone()),
            Some(c) if c.permission != d.permission => to_update.push(d.clone()),
            _ => {}
        }
    }

    for c in current {
        if !desired.iter().any(|d| d.slug == c.slug) {
            to_remove.push(c.slug.clone());
        }
    }

    TeamDiff {
        repo: repo.to_string(),
        to_add,
        to_update,
        to_remove,
    }
}

async fn list(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Listing teams for {} repositories...",
        style("[..]").dim(),
        repos.len()
    );

    println!();
    println!(
        "  {} {}",
        style(format!("{:<40}", "Repository")).bold().underlined(),
        style("Teams").bold().underlined(),
    );
    println!("  {}", style("\u{2500}".repeat(70)).dim());

    for repo_name in &repos {
        let teams = client.list_repo_teams(repo_name).await?;

        let summary = if teams.is_empty() {
            style("(none)").dim().to_string()
        } else {
            teams
                .iter()
                .map(|t| format!("{} ({})", t.slug, t.permission))
                .collect::<Vec<_>>()
                .join(", ")
        };

        println!("  {:<40} {}", repo_name, summary);
    }

    Ok(())
}

async fn build_diffs(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<TeamDiff>> {
    let sys_id = system.ok_or_else(|| {
        anyhow::anyhow!(
            "--system is required for teams plan/apply (teams are configured per system)"
        )
    })?;

    let desired = teams_for_system(manifest, sys_id);
    if desired.is_empty() {
        anyhow::bail!("No teams configured for system '{}' in ward.toml", sys_id);
    }

    let repos = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Scanning teams for {} repositories...",
        style("[..]").dim(),
        repos.len()
    );

    let mut diffs = Vec::new();

    for repo_name in &repos {
        let current = client.list_repo_teams(repo_name).await?;
        diffs.push(diff_teams(repo_name, desired, &current));
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
            style("ward teams apply").cyan().bold()
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
            "\n  {} All team access is up to date.",
            style("[ok]").green()
        );
        return Ok(());
    }

    print_diff_table(&diffs);

    if !yes {
        let proceed = Confirm::new()
            .with_prompt(format!(
                "  Apply team changes to {needs_changes} repositories?"
            ))
            .default(false)
            .interact()?;

        if !proceed {
            println!("  Aborted.");
            return Ok(());
        }
    }

    println!();
    println!("  {} Applying team changes...", style("[..]").dim());

    let mut succeeded = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    for diff in diffs.iter().filter(|d| d.has_changes()) {
        for team in &diff.to_add {
            match client
                .add_team_to_repo(&diff.repo, &team.slug, &team.permission)
                .await
            {
                Ok(()) => {
                    println!(
                        "  {} {}: added {} ({})",
                        style("[ok]").green(),
                        diff.repo,
                        team.slug,
                        team.permission
                    );
                    succeeded += 1;
                }
                Err(e) => {
                    println!(
                        "  {} {}: failed to add {}: {}",
                        style("[!!]").red(),
                        diff.repo,
                        team.slug,
                        e
                    );
                    failed.push((diff.repo.clone(), e.to_string()));
                }
            }
        }

        for team in &diff.to_update {
            match client
                .add_team_to_repo(&diff.repo, &team.slug, &team.permission)
                .await
            {
                Ok(()) => {
                    println!(
                        "  {} {}: updated {} -> {}",
                        style("[ok]").green(),
                        diff.repo,
                        team.slug,
                        team.permission
                    );
                    succeeded += 1;
                }
                Err(e) => {
                    println!(
                        "  {} {}: failed to update {}: {}",
                        style("[!!]").red(),
                        diff.repo,
                        team.slug,
                        e
                    );
                    failed.push((diff.repo.clone(), e.to_string()));
                }
            }
        }

        for slug in &diff.to_remove {
            match client.remove_team_from_repo(&diff.repo, slug).await {
                Ok(()) => {
                    println!(
                        "  {} {}: removed {}",
                        style("[ok]").green(),
                        diff.repo,
                        slug
                    );
                    succeeded += 1;
                }
                Err(e) => {
                    println!(
                        "  {} {}: failed to remove {}: {}",
                        style("[!!]").red(),
                        diff.repo,
                        slug,
                        e
                    );
                    failed.push((diff.repo.clone(), e.to_string()));
                }
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!(
            "  {} {} changes applied successfully.",
            style("[ok]").green(),
            succeeded
        );
    } else {
        println!(
            "  {} {} succeeded, {} failed:",
            style("[!!]").yellow(),
            succeeded,
            failed.len()
        );
        for (repo, err) in &failed {
            println!("    {} {}: {}", style("[!!]").red(), repo, err);
        }
    }

    Ok(())
}

async fn audit(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let sys_id = system.unwrap_or("default");
    let desired = teams_for_system(manifest, sys_id);
    let repos = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Auditing team access for {} repositories...",
        style("[..]").dim(),
        repos.len()
    );

    use tabled::builder::Builder;
    use tabled::settings::object::{Columns, Rows};
    use tabled::settings::{Alignment, Modify, Style};

    let mut builder = Builder::default();
    builder.push_record(["Status", "Repository", "Teams"]);

    let mut total_ok = 0;
    let mut total_issues = 0;

    for repo_name in &repos {
        let teams = client.list_repo_teams(repo_name).await?;

        let all_desired_present = desired.iter().all(|d| {
            teams
                .iter()
                .any(|t| t.slug == d.slug && t.permission == d.permission)
        });

        if all_desired_present && !desired.is_empty() {
            total_ok += 1;
        } else if !desired.is_empty() {
            total_issues += 1;
        }

        let indicator = if desired.is_empty() || all_desired_present {
            style("[ok]").green().to_string()
        } else {
            style("[!!]").red().to_string()
        };

        let summary = if teams.is_empty() {
            "(none)".to_string()
        } else {
            teams
                .iter()
                .map(|t| format!("{} ({})", t.slug, t.permission))
                .collect::<Vec<_>>()
                .join(", ")
        };

        builder.push_record([indicator, repo_name.clone(), summary]);
    }

    let table = builder
        .build()
        .with(Style::blank())
        .with(
            Modify::new(Rows::first()).with(tabled::settings::Format::content(|s| {
                format!("{}", style(s).bold().underlined())
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
        "  Summary: {} compliant, {} need attention",
        style(total_ok).green().bold(),
        if total_issues > 0 {
            style(total_issues).red().bold()
        } else {
            style(total_issues).green().bold()
        }
    );

    Ok(())
}

fn print_diff_table(diffs: &[TeamDiff]) {
    println!();
    println!("  {}", style("Teams Plan").bold().cyan());
    println!("  {}", style("\u{2500}".repeat(60)).dim());

    for diff in diffs {
        if diff.has_changes() {
            println!(
                "  {} {} ({} changes)",
                style("[!!]").yellow(),
                style(&diff.repo).bold(),
                diff.change_count()
            );
            for team in &diff.to_add {
                println!(
                    "     {} add: {} ({})",
                    style("+").green(),
                    team.slug,
                    team.permission
                );
            }
            for team in &diff.to_update {
                println!(
                    "     {} update: {} -> {}",
                    style("~").yellow(),
                    team.slug,
                    team.permission
                );
            }
            for slug in &diff.to_remove {
                println!("     {} remove: {}", style("-").red(), slug);
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

    #[test]
    fn test_team_config_parsing() {
        let toml_str = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
            teams = [
                { slug = "developers", permission = "push" },
                { slug = "devops", permission = "admin" },
            ]
        "#;
        let m: crate::config::Manifest = toml::from_str(toml_str).unwrap();
        let teams = teams_for_system(&m, "be");
        assert_eq!(teams.len(), 2);
        assert_eq!(teams[0].slug, "developers");
        assert_eq!(teams[0].permission, "push");
        assert_eq!(teams[1].slug, "devops");
        assert_eq!(teams[1].permission, "admin");
    }

    #[test]
    fn test_team_access_diff() {
        let desired = vec![
            TeamAccess {
                slug: "developers".to_string(),
                permission: "push".to_string(),
            },
            TeamAccess {
                slug: "devops".to_string(),
                permission: "admin".to_string(),
            },
        ];

        let current = vec![
            Team {
                id: 1,
                name: "Developers".to_string(),
                slug: "developers".to_string(),
                description: None,
                permission: "pull".to_string(),
                privacy: "closed".to_string(),
            },
            Team {
                id: 2,
                name: "Old Team".to_string(),
                slug: "old-team".to_string(),
                description: None,
                permission: "push".to_string(),
                privacy: "closed".to_string(),
            },
        ];

        let diff = diff_teams("my-repo", &desired, &current);
        assert_eq!(diff.repo, "my-repo");
        assert_eq!(diff.to_add.len(), 1);
        assert_eq!(diff.to_add[0].slug, "devops");
        assert_eq!(diff.to_update.len(), 1);
        assert_eq!(diff.to_update[0].slug, "developers");
        assert_eq!(diff.to_update[0].permission, "push");
        assert_eq!(diff.to_remove.len(), 1);
        assert_eq!(diff.to_remove[0], "old-team");
    }

    #[test]
    fn test_team_config_empty_default() {
        let toml_str = r#"
            [org]
            name = "org"
            [[systems]]
            id = "be"
            name = "Backend"
        "#;
        let m: crate::config::Manifest = toml::from_str(toml_str).unwrap();
        let teams = teams_for_system(&m, "be");
        assert!(teams.is_empty());
    }

    #[test]
    fn test_team_diff_no_changes() {
        let desired = vec![TeamAccess {
            slug: "devs".to_string(),
            permission: "push".to_string(),
        }];

        let current = vec![Team {
            id: 1,
            name: "Devs".to_string(),
            slug: "devs".to_string(),
            description: None,
            permission: "push".to_string(),
            privacy: "closed".to_string(),
        }];

        let diff = diff_teams("repo", &desired, &current);
        assert!(!diff.has_changes());
    }
}
