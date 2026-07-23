use anyhow::Result;
use clap::Args;

use crate::config::Manifest;
use crate::github::Client;
use crate::reconcile::unified::{self, UnifiedOptions, UnifiedReport};

pub(crate) struct CategoryRun<'a> {
    pub system: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub json: bool,
    pub command: &'a str,
    pub title: &'a str,
}

/// Preview the desired manifest state across existing repositories.
#[derive(Args)]
pub struct PlanCommand {
    /// Compatibility flag; canonical planning already checks every configured system by default
    #[arg(long)]
    all: bool,

    /// Limit to one or more categories (repeatable). Valid: repository, files,
    /// security, rulesets, branch-protection, actions, environments, access,
    /// integrations.
    #[arg(long = "category", value_name = "CATEGORY")]
    categories: Vec<String>,

    /// Allow planning high-impact repository changes (visibility, archive)
    #[arg(long)]
    allow_high_impact: bool,
}

impl PlanCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
        json: bool,
    ) -> Result<()> {
        let _ = self.all;
        let options = UnifiedOptions {
            categories: unified::parse_categories(&self.categories)?,
            allow_high_impact: self.allow_high_impact,
            verify: true,
        };
        run_canonical_plan(
            client,
            manifest,
            options,
            CategoryRun {
                system,
                repo,
                json,
                command: "plan",
                title: "Ward Plan",
            },
        )
        .await
        .map(|_| ())
    }
}

pub(crate) fn require_canonical_categories(manifest: &Manifest, command: &str) -> Result<()> {
    let has_categories = !manifest.categories.is_empty()
        || manifest
            .systems
            .iter()
            .any(|system| !system.categories.is_empty());
    if !has_categories {
        anyhow::bail!(
            "`ward {command}` requires at least one configured category. Run `ward init` or `ward import OWNER/REPO`, then edit the manifest."
        );
    }
    Ok(())
}

/// Run category planning and render the standard unified report.
pub(crate) async fn run_canonical_plan(
    client: &Client,
    manifest: &Manifest,
    options: UnifiedOptions,
    run: CategoryRun<'_>,
) -> Result<UnifiedReport> {
    require_canonical_categories(manifest, run.command)?;

    let repos = unified::resolve_target_repos(client, manifest, run.system, run.repo).await?;
    let report = if repos.is_empty() {
        UnifiedReport::from_repos(Vec::new())
    } else {
        unified::plan(client, manifest, &repos, &options).await?
    };

    if run.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if repos.is_empty() {
        println!("  No matching repositories found.");
    } else {
        unified::render_report(&report, run.title);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn plan_accepts_category_filtering_and_high_impact_opt_in() {
        let cli = crate::cli::Cli::parse_from([
            "ward",
            "plan",
            "--category",
            "files",
            "--allow-high-impact",
        ]);
        let crate::cli::Command::Plan(command) = cli.command else {
            panic!("expected plan command");
        };

        assert_eq!(command.categories, ["files"]);
        assert!(command.allow_high_impact);
    }
}
