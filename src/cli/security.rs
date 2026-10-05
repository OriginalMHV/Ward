use anyhow::Result;
use clap::Args;
use console::style;

use super::output::{ok_icon, print_table};
use crate::config::Manifest;
use crate::engine::audit_log::AuditLog;
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
        json: bool,
    ) -> Result<()> {
        self.run_with_audit(client, manifest, system, repo, json, AuditLog::new)
            .await
    }

    /// As [`Self::run`], with the audit log opened by `open_audit` when an apply starts.
    pub async fn run_with_audit(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
        json: bool,
        open_audit: impl FnOnce() -> Result<AuditLog>,
    ) -> Result<()> {
        match &self.action {
            SecurityAction::Plan => crate::cli::plan::run_canonical_plan(
                client,
                manifest,
                options(true),
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json,
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
                open_audit,
                crate::cli::plan::CategoryRun {
                    system,
                    repo,
                    json,
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

    let mut builder = Builder::default();
    builder.push_record(["Repository", "Dep.A", "Dep.SU", "Secret", "AI", "Push"]);

    let mut total_ok = 0;
    let mut total_issues = 0;

    let states = crate::reconcile::map_buffered(&repositories, |repository| async {
        client
            .get_security_state_with_repo_data(
                &repository.name,
                repository.security_and_analysis.as_ref(),
            )
            .await
    })
    .await;

    for (repository, state) in repositories.iter().zip(states) {
        let state = state?;
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

        let icons: Vec<String> = features.iter().map(|&enabled| ok_icon(enabled)).collect();

        builder.push_record([
            repository.name.clone(),
            icons[0].clone(),
            icons[1].clone(),
            icons[2].clone(),
            icons[3].clone(),
            icons[4].clone(),
        ]);
    }

    println!();
    print_table(builder);

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

    #[test]
    fn global_json_flag_is_accepted_after_the_focused_subcommand() {
        use clap::Parser;
        let cli = crate::cli::Cli::parse_from(["ward", "security", "plan", "--json"]);
        assert!(cli.json);
    }
}
