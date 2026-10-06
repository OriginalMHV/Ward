//! Deprecated per-category commands.
//!
//! `ward security`, `rulesets`, `protection` and `commit` were focused wrappers
//! over one category each. They stay for 0.5.x as hidden aliases that warn and
//! then run the same code as `ward plan`, `apply` and `audit`.

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::cli::audit::{AuditCategory, AuditCommand};
use crate::cli::plan::CategoryRun;
use crate::config::Manifest;
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::reconcile::unified::{Category, UnifiedOptions};

/// The actions of a legacy per-category command.
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyAction {
    /// Show what would change
    Plan,

    /// Apply the changes
    Apply {
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,

        /// Skip the post-apply verification step
        #[arg(long)]
        skip_verify: bool,
    },

    /// Show the current state
    Audit,
}

/// The arguments of a hidden legacy command such as `ward security plan`.
#[derive(Args, Debug)]
pub struct LegacyArgs {
    #[command(subcommand)]
    action: LegacyAction,
}

impl LegacyArgs {
    /// Bind the parsed action to the legacy command name and its category.
    pub fn into_legacy(self, command: &'static str, category: Category) -> LegacyCategory {
        LegacyCategory {
            command,
            category,
            action: self.action,
        }
    }
}

/// One legacy command invocation, mapped onto the generic category commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCategory {
    pub command: &'static str,
    pub category: Category,
    pub action: LegacyAction,
}

impl LegacyCategory {
    fn action_name(&self) -> &'static str {
        match self.action {
            LegacyAction::Plan => "plan",
            LegacyAction::Apply { .. } => "apply",
            LegacyAction::Audit => "audit",
        }
    }

    /// The `ward ...` invocation that replaces this one.
    pub fn replacement(&self) -> String {
        format!(
            "ward {} --category {}",
            self.action_name(),
            self.category.stable_name()
        )
    }

    /// The deprecation warning, without a trailing newline.
    pub fn warning(&self) -> String {
        format!(
            "warning: 'ward {} {}' is deprecated and will be removed in 0.6.0; use '{}'",
            self.command,
            self.action_name(),
            self.replacement()
        )
    }

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
        eprintln!("{}", self.warning());
        let options = |verify| UnifiedOptions {
            categories: vec![self.category],
            allow_high_impact: false,
            verify,
        };
        match self.action {
            LegacyAction::Plan => crate::cli::plan::run_canonical_plan(
                client,
                manifest,
                options(true),
                CategoryRun {
                    system,
                    repo,
                    json,
                    command: "plan",
                    title: "Ward Plan",
                },
            )
            .await
            .map(|_| ()),
            LegacyAction::Apply { yes, skip_verify } => crate::cli::apply::run_canonical_apply(
                client,
                manifest,
                yes,
                options(!skip_verify),
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
            .map(|_| ()),
            LegacyAction::Audit => {
                let Some(section) = self.audit_section() else {
                    bail!(
                        "`ward {} audit` has no replacement. Use `ward plan --category {}` to compare the state with the manifest.",
                        self.command,
                        self.category.stable_name()
                    );
                };
                AuditCommand::for_section(section, json)
                    .run(client, manifest, system, repo)
                    .await
            }
        }
    }

    fn audit_section(&self) -> Option<AuditCategory> {
        match self.category {
            Category::Security => Some(AuditCategory::Security),
            Category::Rulesets => Some(AuditCategory::Rulesets),
            Category::BranchProtection => Some(AuditCategory::BranchProtection),
            Category::Access => Some(AuditCategory::Access),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};

    fn legacy(args: &[&str]) -> LegacyCategory {
        let cli = Cli::parse_from(args);
        match cli.command {
            Command::Security(args) => args.into_legacy("security", Category::Security),
            Command::Rulesets(args) => args.into_legacy("rulesets", Category::Rulesets),
            Command::Protection(args) => args.into_legacy("protection", Category::BranchProtection),
            Command::Commit(args) => args.into_legacy("commit", Category::Files),
            _ => panic!("expected a legacy command"),
        }
    }

    #[test]
    fn legacy_commands_keep_their_subcommands() {
        assert_eq!(
            legacy(&["ward", "security", "plan", "--repo", "target"]).action,
            LegacyAction::Plan
        );
        assert_eq!(
            legacy(&["ward", "rulesets", "audit"]).action,
            LegacyAction::Audit
        );
    }

    #[test]
    fn legacy_apply_keeps_yes_and_skip_verify() {
        for flag in ["--yes", "-y"] {
            let command = legacy(&["ward", "protection", "apply", flag, "--skip-verify"]);
            assert_eq!(
                command.action,
                LegacyAction::Apply {
                    yes: true,
                    skip_verify: true
                }
            );
        }
    }

    #[test]
    fn legacy_apply_without_flags_prompts_and_verifies() {
        let command = legacy(&["ward", "commit", "apply"]);
        assert_eq!(
            command.action,
            LegacyAction::Apply {
                yes: false,
                skip_verify: false
            }
        );
    }

    #[test]
    fn each_legacy_command_maps_to_its_category() {
        for (word, category) in [
            ("security", Category::Security),
            ("rulesets", Category::Rulesets),
            ("protection", Category::BranchProtection),
            ("commit", Category::Files),
        ] {
            assert_eq!(legacy(&["ward", word, "plan"]).category, category, "{word}");
        }
    }

    #[test]
    fn warning_names_the_replacement_and_the_removal_release() {
        assert_eq!(
            legacy(&["ward", "security", "plan"]).warning(),
            "warning: 'ward security plan' is deprecated and will be removed in 0.6.0; use 'ward plan --category security'"
        );
        assert_eq!(
            legacy(&["ward", "protection", "apply", "-y"]).warning(),
            "warning: 'ward protection apply' is deprecated and will be removed in 0.6.0; use 'ward apply --category branch-protection'"
        );
        assert_eq!(
            legacy(&["ward", "rulesets", "audit"]).warning(),
            "warning: 'ward rulesets audit' is deprecated and will be removed in 0.6.0; use 'ward audit --category rulesets'"
        );
        assert_eq!(
            legacy(&["ward", "commit", "plan"]).warning(),
            "warning: 'ward commit plan' is deprecated and will be removed in 0.6.0; use 'ward plan --category files'"
        );
    }

    #[test]
    fn audit_exists_only_for_categories_with_an_audit_section() {
        assert!(
            legacy(&["ward", "security", "audit"])
                .audit_section()
                .is_some()
        );
        assert!(
            legacy(&["ward", "commit", "audit"])
                .audit_section()
                .is_none()
        );
    }

    #[test]
    fn legacy_commands_are_hidden_from_help() {
        use clap::CommandFactory;
        let help = Cli::command().render_long_help().to_string();
        for word in ["security", "rulesets", "protection", "commit"] {
            assert!(
                !help.lines().any(|line| line.trim_start().starts_with(word)),
                "{word} is listed in help"
            );
        }
    }
}
