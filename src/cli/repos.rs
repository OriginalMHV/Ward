use anyhow::Result;
use clap::Args;
use console::style;

use super::args::{ListTargetArgs, OutputArgs};
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
    List {
        #[command(flatten)]
        target: ListTargetArgs,

        #[command(flatten)]
        output: OutputArgs,
    },

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
            ReposAction::List { .. } => None,
        }
    }

    /// The organization override of `repos list`.
    pub fn org(&self) -> Option<&str> {
        match &self.action {
            ReposAction::List { target, .. } => target.org.as_deref(),
            ReposAction::Inspect { .. } => None,
        }
    }

    /// Reject the removed `repos inspect`. This needs no token or manifest.
    pub fn precheck(&self) -> Result<()> {
        match &self.action {
            ReposAction::List { .. } => Ok(()),
            ReposAction::Inspect { .. } => {
                anyhow::bail!("{}", self.removed_hint().unwrap_or_default())
            }
        }
    }

    pub async fn run(&self, client: &Client, manifest: &Manifest) -> Result<()> {
        match &self.action {
            ReposAction::List { target, output } => {
                list_repos(client, manifest, target.system.as_deref(), output.is_json()).await
            }
            ReposAction::Inspect { .. } => self.precheck(),
        }
    }
}

#[derive(serde::Serialize)]
struct RepoRow<'a> {
    name: &'a str,
    language: Option<&'a str>,
    visibility: &'a str,
    default_branch: &'a str,
}

async fn list_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    json: bool,
) -> Result<()> {
    let repos = if system.is_some() {
        unified::resolve_target_repos(client, manifest, system, None).await?
    } else {
        client.list_repos().await?
    };

    if json {
        let rows: Vec<RepoRow<'_>> = repos
            .iter()
            .map(|r| RepoRow {
                name: &r.name,
                language: r.language.as_deref(),
                visibility: &r.visibility,
                default_branch: &r.default_branch,
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

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
