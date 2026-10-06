use anyhow::Result;
use clap::Args;
use console::style;

use super::output::print_table;
use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified;

#[derive(Args)]
pub struct ReposCommand {
    #[command(subcommand)]
    action: ReposAction,
}

#[derive(clap::Subcommand)]
enum ReposAction {
    /// List repositories with metadata
    List,

    /// Removed. Use `ward audit --repo NAME`
    #[command(hide = true)]
    Inspect {
        #[arg(num_args = 0.., allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

impl ReposCommand {
    /// The replacement hint for a removed subcommand, which needs no token or manifest.
    pub fn removed_hint(&self) -> Option<&'static str> {
        match self.action {
            ReposAction::Inspect { .. } => {
                Some("`ward repos inspect` was removed. Use `ward audit --repo NAME`.")
            }
            ReposAction::List => None,
        }
    }

    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
    ) -> Result<()> {
        match &self.action {
            ReposAction::List => list_repos(client, manifest, system).await,
            ReposAction::Inspect { .. } => {
                anyhow::bail!("{}", self.removed_hint().unwrap_or_default())
            }
        }
    }
}

async fn list_repos(client: &Client, manifest: &Manifest, system: Option<&str>) -> Result<()> {
    let repos = if system.is_some() {
        unified::resolve_target_repos(client, manifest, system, None).await?
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

    let mut builder = Builder::default();
    builder.push_record(["Repository", "Language", "Visibility", "Branch"]);
    for row in &rows {
        builder.push_record(row);
    }

    print_table(builder);

    Ok(())
}
