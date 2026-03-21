use anyhow::Result;
use clap::Args;
use console::style;

use crate::config::Manifest;
use crate::github::Client;

#[derive(Args)]
pub struct CommitCommand {
    #[command(subcommand)]
    action: CommitAction,
}

#[derive(clap::Subcommand)]
enum CommitAction {
    /// Preview what files would be committed
    Plan {
        /// Template name (e.g., dependabot, codeql, dependency-submission)
        #[arg(long)]
        template: String,
    },

    /// Commit template files and create PRs
    Apply {
        /// Template name (e.g., dependabot, codeql, dependency-submission)
        #[arg(long)]
        template: String,

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
            CommitAction::Plan { template } => {
                plan(client, manifest, system, repo, template).await
            }
            CommitAction::Apply { template, yes } => {
                apply(client, manifest, system, repo, template, *yes).await
            }
        }
    }
}

async fn plan(
    _client: &Client,
    _manifest: &Manifest,
    _system: Option<&str>,
    _repo: Option<&str>,
    template: &str,
) -> Result<()> {
    println!();
    println!(
        "  {} Commit plan for template: {}",
        style("📋").bold(),
        style(template).cyan().bold()
    );
    println!(
        "  {} This feature is coming in Phase 2.",
        style("🚧").yellow()
    );
    Ok(())
}

async fn apply(
    _client: &Client,
    _manifest: &Manifest,
    _system: Option<&str>,
    _repo: Option<&str>,
    template: &str,
    _yes: bool,
) -> Result<()> {
    println!();
    println!(
        "  {} Commit apply for template: {}",
        style("📋").bold(),
        style(template).cyan().bold()
    );
    println!(
        "  {} This feature is coming in Phase 2.",
        style("🚧").yellow()
    );
    Ok(())
}
