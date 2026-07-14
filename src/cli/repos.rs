use anyhow::Result;
use clap::Args;
use console::style;
use tabled::settings::Style;

use crate::config::Manifest;
use crate::github::Client;

#[derive(Args)]
pub struct ReposCommand {
    #[command(subcommand)]
    action: ReposAction,
}

#[derive(clap::Subcommand)]
enum ReposAction {
    /// List repositories with metadata
    List,

    /// Inspect a single repository in detail
    Inspect {
        /// Repository name (without org prefix)
        name: String,
    },
}

impl ReposCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
    ) -> Result<()> {
        match &self.action {
            ReposAction::List => list_repos(client, manifest, system).await,
            ReposAction::Inspect { name } => inspect_repo(client, name).await,
        }
    }
}

async fn list_repos(client: &Client, manifest: &Manifest, system: Option<&str>) -> Result<()> {
    let repos = if let Some(sys) = system {
        let excludes = manifest.exclude_patterns_for_system(sys);
        let explicit = manifest.explicit_repos_for_system(sys);
        client
            .list_repos_for_system(
                sys,
                manifest.matches_prefix_for_system(sys),
                &excludes,
                &explicit,
            )
            .await?
    } else {
        client.list_repos().await?
    };

    if repos.is_empty() {
        println!("  No repositories found.");
        return Ok(());
    }

    let rows: Vec<[String; 4]> = repos
        .iter()
        .map(|r| {
            [
                r.name.clone(),
                r.language.clone().unwrap_or_else(|| "-".to_owned()),
                r.visibility.clone(),
                r.default_branch.clone(),
            ]
        })
        .collect();

    println!();
    println!(
        "  {} repositories in {}{}\n",
        style(repos.len()).bold().cyan(),
        style(client.org()).bold(),
        system
            .map(|s| format!(" (system: {s})"))
            .unwrap_or_default()
    );

    use tabled::builder::Builder;
    use tabled::settings::object::{Columns, Rows};
    use tabled::settings::{Alignment, Modify};

    let mut builder = Builder::default();
    builder.push_record(["Repository", "Language", "Visibility", "Branch"]);
    for row in &rows {
        builder.push_record(row);
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

    for line in table.lines() {
        println!("  {line}");
    }

    Ok(())
}

async fn inspect_repo(client: &Client, name: &str) -> Result<()> {
    let repo = client.get_repo(name).await?;
    let security = client.get_security_state(name).await?;

    println!();
    println!("  {} {}", style("Repository:").bold(), repo.full_name);
    println!(
        "  {} {}",
        style("Description:").bold(),
        repo.description.as_deref().unwrap_or("-")
    );
    println!(
        "  {} {}",
        style("Language:").bold(),
        repo.language.as_deref().unwrap_or("-")
    );
    println!("  {} {}", style("Visibility:").bold(), repo.visibility);
    println!(
        "  {} {}",
        style("Default Branch:").bold(),
        repo.default_branch
    );
    println!("  {} {}", style("Archived:").bold(), repo.archived);

    println!();
    println!("  {}", style("Security Status:").bold().underlined());
    print_feature("  Dependabot Alerts", security.dependabot_alerts);
    print_feature(
        "  Dependabot Security Updates",
        security.dependabot_security_updates,
    );
    print_feature("  Secret Scanning", security.secret_scanning);
    print_feature(
        "  Secret Scanning AI",
        security.secret_scanning_ai_detection,
    );
    print_feature("  Push Protection", security.push_protection);

    Ok(())
}

fn print_feature(name: &str, enabled: bool) {
    let icon = if enabled {
        style("[ok]").green()
    } else {
        style("[!!]").red()
    };
    println!(
        "{name}: {icon} {}",
        if enabled { "enabled" } else { "disabled" }
    );
}
