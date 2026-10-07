use std::io::IsTerminal;

use anyhow::{Result, bail};
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::cli::args::{CategoryArgs, OutputArgs, TargetArgs};
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
    #[command(flatten)]
    category: CategoryArgs,

    /// Allow high-impact repository changes (visibility, archive)
    #[arg(long)]
    allow_high_impact: bool,

    /// Skip the post-apply verification step
    #[arg(long)]
    skip_verify: bool,

    /// Skip the confirmation prompt (required with --format json)
    #[arg(short = 'y', long)]
    yes: bool,

    #[command(flatten)]
    pub target: TargetArgs,

    #[command(flatten)]
    output: OutputArgs,
}

impl ApplyCommand {
    pub async fn run(&self, client: &Client, manifest: &Manifest) -> Result<()> {
        self.run_with_audit(client, manifest, AuditLog::new).await
    }

    /// As [`Self::run`], with the audit log opened by `open_audit` when an apply starts.
    pub async fn run_with_audit(
        &self,
        client: &Client,
        manifest: &Manifest,
        open_audit: impl FnOnce() -> Result<AuditLog>,
    ) -> Result<()> {
        let options = UnifiedOptions {
            categories: unified::select_categories(&self.category.categories),
            allow_high_impact: self.allow_high_impact,
            verify: !self.skip_verify,
        };
        run_canonical_apply(
            client,
            manifest,
            self.yes,
            options,
            open_audit,
            CategoryRun {
                system: self.target.system.as_deref(),
                repo: self.target.repo.as_deref(),
                json: self.output.is_json(),
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
    ensure_can_confirm(yes, std::io::stdin().is_terminal())?;

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
        crate::cli::render::render_report(&prepared.report(), "Ward Plan (to apply)");
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
        crate::cli::render::render_report(&report, run.title);
    }

    if report.has_failures() {
        let (failed, blocked) = report.category_problem_counts();
        return Err(Outcome::ApplyFailed(crate::outcome::apply_summary(failed, blocked)).into());
    }

    Ok(report)
}

/// Without `--yes`, apply must ask. Refuse when nobody can answer.
fn ensure_can_confirm(yes: bool, interactive: bool) -> Result<()> {
    if !yes && !interactive {
        bail!("refusing to prompt in a non-interactive session; pass --yes");
    }
    Ok(())
}

fn validate_confirmation_mode(json: bool, yes: bool) -> Result<()> {
    if json && !yes {
        bail!(
            "`ward apply --format json` requires `--yes`; JSON output must not bypass confirmation"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::validate_confirmation_mode;

    #[test]
    fn non_interactive_apply_without_yes_is_refused() {
        let error = super::ensure_can_confirm(false, false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "refusing to prompt in a non-interactive session; pass --yes"
        );
        assert!(
            error.downcast_ref::<crate::outcome::Outcome>().is_none(),
            "this is an operational error and exits with 2"
        );
    }

    #[test]
    fn apply_may_run_when_confirmed_or_interactive() {
        super::ensure_can_confirm(true, false).unwrap();
        super::ensure_can_confirm(false, true).unwrap();
    }

    #[test]
    fn apply_accepts_short_yes_and_skip_verify() {
        let cli = crate::cli::Cli::parse_from([
            "ward",
            "apply",
            "-y",
            "--skip-verify",
            "--category",
            "files,security",
        ]);
        let crate::cli::Command::Apply(command) = cli.command else {
            panic!("expected apply command");
        };

        assert!(command.yes);
        assert!(command.skip_verify);
        assert_eq!(command.category.categories.len(), 2);
    }

    #[test]
    fn json_apply_requires_explicit_confirmation() {
        let error = validate_confirmation_mode(true, false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`ward apply --format json` requires `--yes`")
        );
    }

    #[test]
    fn explicit_confirmation_allows_json_apply() {
        validate_confirmation_mode(true, true).unwrap();
    }
}
