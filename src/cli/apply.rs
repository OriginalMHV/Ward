use anyhow::{Result, bail};
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::cli::plan::CategoryRun;
use crate::config::Manifest;
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::outcome::Outcome;
use crate::reconcile::unified::{self, UnifiedOptions, UnifiedReport};

/// Apply the desired manifest state to existing repositories.
///
/// Apply plans every selected category first, then mutates in a safe order:
/// repository/general, files (via a dedicated branch and pull request),
/// security, actions, environments, access, integrations, rulesets, and
/// classic branch protection. It never creates, renames, transfers, or
/// deletes repositories.
#[derive(Args)]
pub struct ApplyCommand {
    /// Limit to one or more categories (repeatable). Valid: repository, files,
    /// security, rulesets, branch-protection, actions, environments, access,
    /// integrations.
    #[arg(long = "category", value_name = "CATEGORY")]
    categories: Vec<String>,

    /// Allow high-impact repository changes (visibility, archive)
    #[arg(long)]
    allow_high_impact: bool,

    /// Skip the confirmation prompt (required with --json)
    #[arg(long)]
    yes: bool,
}

impl ApplyCommand {
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
        let options = UnifiedOptions {
            categories: unified::parse_categories(&self.categories)?,
            allow_high_impact: self.allow_high_impact,
            verify: true,
        };
        run_canonical_apply(
            client,
            manifest,
            self.yes,
            options,
            open_audit,
            CategoryRun {
                system,
                repo,
                json,
                command: "apply",
                title: "Ward Apply",
            },
        )
        .await
        .map(|_| ())
    }
}

/// Run category application and render the standard unified report.
pub(crate) async fn run_canonical_apply(
    client: &Client,
    manifest: &Manifest,
    yes: bool,
    options: UnifiedOptions,
    open_audit: impl FnOnce() -> Result<AuditLog>,
    run: CategoryRun<'_>,
) -> Result<UnifiedReport> {
    crate::cli::plan::require_canonical_categories(manifest, run.command)?;
    validate_confirmation_mode(run.json, yes)?;

    let repos = unified::resolve_target_repos(client, manifest, run.system, run.repo).await?;
    if run.repo.is_some() {
        unified::reject_archived_explicit_target(&repos)?;
    }
    if repos.is_empty() {
        let report = UnifiedReport::from_repos(Vec::new());
        if run.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!("  No matching repositories found.");
        }
        return Ok(report);
    }

    let prepared = unified::prepare_apply(client, manifest, &repos, &options).await?;

    if !yes {
        unified::render_report(&prepared.report(), "Ward Plan (to apply)");
        println!();
        println!(
            "  {} Apply this plan to {} repositor{}?",
            style("[!]").yellow().bold(),
            style(repos.len()).bold(),
            if repos.len() == 1 { "y" } else { "ies" }
        );
        let proceed = Confirm::new()
            .with_prompt("  Proceed?")
            .default(false)
            .interact()?;
        if !proceed {
            println!("  Aborted.");
            return Ok(UnifiedReport::from_repos(Vec::new()));
        }
    }

    let audit = open_audit()?;
    let report = unified::apply_prepared(client, manifest, prepared, &options, &audit).await;

    if run.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        unified::render_report(&report, run.title);
    }

    if report.has_failures() {
        let (failed, blocked) = report.category_problem_counts();
        return Err(Outcome::ApplyFailed(crate::outcome::apply_summary(failed, blocked)).into());
    }

    Ok(report)
}

fn validate_confirmation_mode(json: bool, yes: bool) -> Result<()> {
    if json && !yes {
        bail!("`ward apply --json` requires `--yes`; JSON output must not bypass confirmation");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_confirmation_mode;

    #[test]
    fn json_apply_requires_explicit_confirmation() {
        let error = validate_confirmation_mode(true, false).unwrap_err();
        assert!(error.to_string().contains("requires `--yes`"));
    }

    #[test]
    fn explicit_confirmation_allows_json_apply() {
        validate_confirmation_mode(true, true).unwrap();
    }
}
