use anyhow::Result;
use clap::Args;
use console::style;

use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified::{self, Category, UnifiedOptions};

#[derive(Args)]
pub struct SecurityCommand {
    #[command(subcommand)]
    action: SecurityAction,
}

#[derive(clap::Subcommand)]
enum SecurityAction {
    /// Show what security changes would be made
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

    /// Audit current security state across repositories
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
            SecurityAction::Plan => crate::cli::plan::run_canonical_plan(
                client,
                manifest,
                options(true),
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "security plan",
                    title: "Ward Security Plan",
                },
            )
            .await
            .map(|_| ()),
            SecurityAction::Apply { yes, skip_verify } => crate::cli::apply::run_canonical_apply(
                client,
                manifest,
                *yes,
                options(!skip_verify),
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json: false,
                    command: "security apply",
                    title: "Ward Security Apply",
                },
            )
            .await
            .map(|_| ()),
            SecurityAction::Audit => audit(client, manifest, system, repo).await,
        }
    }
}

fn options(verify: bool) -> UnifiedOptions {
    UnifiedOptions {
        categories: vec![Category::Security],
        allow_high_impact: false,
        verify,
    }
}

async fn audit(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let repositories = unified::resolve_target_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Auditing {} repositories...",
        style("[..]").bold(),
        repositories.len()
    );

    use tabled::builder::Builder;
    use tabled::settings::object::{Columns, Rows};
    use tabled::settings::{Alignment, Modify, Style};

    let mut builder = Builder::default();
    builder.push_record(["Repository", "Dep.A", "Dep.SU", "Secret", "AI", "Push"]);

    let mut total_ok = 0;
    let mut total_issues = 0;

    for repository in &repositories {
        let state = client.get_security_state(&repository.name).await?;
        let features = [
            state.dependabot_alerts,
            state.dependabot_security_updates,
            state.secret_scanning,
            state.secret_scanning_ai_detection,
            state.push_protection,
        ];

        if features.iter().all(|&feature| feature) {
            total_ok += 1;
        } else {
            total_issues += 1;
        }

        let icons: Vec<String> = features
            .iter()
            .map(|&enabled| {
                if enabled {
                    format!("{}", style("[ok]").green())
                } else {
                    format!("{}", style("[!!]").red())
                }
            })
            .collect();

        builder.push_record([
            repository.name.clone(),
            icons[0].clone(),
            icons[1].clone(),
            icons[2].clone(),
            icons[3].clone(),
            icons[4].clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_security_options_select_only_security() {
        let options = options(true);
        assert_eq!(options.categories, [Category::Security]);
        assert!(options.verify);
        assert!(!options.allow_high_impact);
    }

    #[test]
    fn skip_verify_is_preserved_by_focused_security_apply() {
        assert!(!options(false).verify);
    }
}
