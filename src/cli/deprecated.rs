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

/// The actions of `ward security`, `rulesets`, `protection` and `commit`.
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyClapAction {
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

/// The actions of `ward teams`.
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamsClapAction {
    /// List the teams of each repository
    List,

    /// Show what would change
    Plan,

    /// Apply the changes
    Apply {
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Compare team access with the manifest
    Audit,
}

/// The actions of `ward settings`.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SettingsClapAction {
    /// Show what would change
    Plan {
        /// Removed. Manage the Copilot review ruleset in ward.toml
        #[arg(long, hide = true)]
        ruleset: Option<String>,
    },

    /// Apply the changes
    Apply {
        /// Removed. Manage the Copilot review ruleset in ward.toml
        #[arg(long, hide = true)]
        ruleset: Option<String>,

        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Compare repository settings with the manifest
    Audit,
}

/// What a legacy command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyAction {
    Plan,
    Apply { yes: bool, skip_verify: bool },
    Audit,
    Drift,
}

/// The arguments of a hidden legacy command such as `ward security plan`.
#[derive(Args, Debug)]
pub struct LegacyArgs {
    #[command(subcommand)]
    action: LegacyClapAction,
}

/// The arguments of the hidden `ward teams` command.
#[derive(Args, Debug)]
pub struct TeamsArgs {
    #[command(subcommand)]
    action: TeamsClapAction,
}

/// The manifest entry that replaces `ward settings --ruleset copilot-review`.
pub const COPILOT_REVIEW_SNIPPET: &str = r#"[[categories.rulesets.repository_rulesets]]
name = "Copilot Code Review"
target = "branch"
enforcement = "active"
conditions_json = '{"ref_name":{"include":["~DEFAULT_BRANCH"],"exclude":[]}}'
[[categories.rulesets.repository_rulesets.rules]]
type = "copilot_code_review"
parameters_json = '{"review_on_push":true,"review_draft_pull_requests":false}'"#;

const REPOSITORY_SCOPE_NOTE: &str = "note: the repository category is wider than 'ward settings'. It also covers metadata, custom properties, immutable releases, labels, and prune.";

/// The arguments of the hidden `ward settings` command.
#[derive(Args, Debug)]
pub struct SettingsArgs {
    #[command(subcommand)]
    action: SettingsClapAction,
}

impl SettingsArgs {
    /// Reject the removed `--ruleset` option. This needs no token or manifest.
    pub fn precheck(&self) -> Result<()> {
        let ruleset = match &self.action {
            SettingsClapAction::Plan { ruleset } | SettingsClapAction::Apply { ruleset, .. } => {
                ruleset.as_deref()
            }
            SettingsClapAction::Audit => None,
        };
        let Some(ruleset) = ruleset else {
            return Ok(());
        };
        if ruleset == "copilot-review" {
            bail!(
                "`ward settings --ruleset copilot-review` was removed. Declare the ruleset in ward.toml and run `ward apply --category rulesets`. Set the rulesets category to disposition = \"managed\" and sensitive = true, then add:\n\n{COPILOT_REVIEW_SNIPPET}"
            );
        }
        bail!("`ward settings --ruleset` was removed, and `{ruleset}` is not a known ruleset.");
    }

    /// `settings plan` and `apply` run the repository category. `audit` becomes a drift check.
    pub fn into_legacy(self) -> LegacyCategory {
        let (name, action, note) = match self.action {
            SettingsClapAction::Plan { .. } => {
                ("plan", LegacyAction::Plan, Some(REPOSITORY_SCOPE_NOTE))
            }
            SettingsClapAction::Apply { yes, .. } => (
                "apply",
                LegacyAction::Apply {
                    yes,
                    skip_verify: false,
                },
                Some(REPOSITORY_SCOPE_NOTE),
            ),
            SettingsClapAction::Audit => ("audit", LegacyAction::Drift, None),
        };
        LegacyCategory {
            command: "settings",
            subcommand: name,
            category: Category::Repository,
            action,
            note,
        }
    }
}

const ACCESS_SCOPE_NOTE: &str = "note: the access category also covers collaborators. Ward manages them only when the manifest sets `collaborators`, and it never removes collaborators when the key is absent.";

impl LegacyArgs {
    /// Bind the parsed action to the legacy command name and its category.
    pub fn into_legacy(self, command: &'static str, category: Category) -> LegacyCategory {
        let (name, action) = match self.action {
            LegacyClapAction::Plan => ("plan", LegacyAction::Plan),
            LegacyClapAction::Apply { yes, skip_verify } => {
                ("apply", LegacyAction::Apply { yes, skip_verify })
            }
            LegacyClapAction::Audit => ("audit", LegacyAction::Audit),
        };
        LegacyCategory {
            command,
            subcommand: name,
            category,
            action,
            note: None,
        }
    }
}

impl TeamsArgs {
    /// `teams plan` and `apply` run the access category. `list` becomes an audit
    /// section and `audit` becomes a drift check.
    pub fn into_legacy(self) -> LegacyCategory {
        let (name, action, note) = match self.action {
            TeamsClapAction::List => ("list", LegacyAction::Audit, None),
            TeamsClapAction::Plan => ("plan", LegacyAction::Plan, Some(ACCESS_SCOPE_NOTE)),
            TeamsClapAction::Apply { yes } => (
                "apply",
                LegacyAction::Apply {
                    yes,
                    skip_verify: false,
                },
                Some(ACCESS_SCOPE_NOTE),
            ),
            TeamsClapAction::Audit => ("audit", LegacyAction::Drift, None),
        };
        LegacyCategory {
            command: "teams",
            subcommand: name,
            category: Category::Access,
            action,
            note,
        }
    }
}

/// One legacy command invocation, mapped onto the generic category commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCategory {
    pub command: &'static str,
    pub subcommand: &'static str,
    pub category: Category,
    pub action: LegacyAction,
    note: Option<&'static str>,
}

impl LegacyCategory {
    fn replacement_name(&self) -> &'static str {
        match self.action {
            LegacyAction::Plan => "plan",
            LegacyAction::Apply { .. } => "apply",
            LegacyAction::Audit => "audit",
            LegacyAction::Drift => "drift",
        }
    }

    /// The `ward ...` invocation that replaces this one.
    pub fn replacement(&self) -> String {
        format!(
            "ward {} --category {}",
            self.replacement_name(),
            self.category.stable_name()
        )
    }

    /// The deprecation warning, without a trailing newline.
    pub fn warning(&self) -> String {
        format!(
            "warning: 'ward {} {}' is deprecated and will be removed in 0.6.0; use '{}'",
            self.command,
            self.subcommand,
            self.replacement()
        )
    }

    /// Print the deprecation warning, and any scope note, to stderr.
    pub fn announce(&self) {
        eprintln!("{}", self.warning());
        if let Some(note) = self.note {
            eprintln!("{note}");
        }
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
        self.announce();
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
            LegacyAction::Drift => {
                crate::cli::drift::run_drift(
                    client,
                    manifest,
                    system,
                    repo,
                    json,
                    vec![self.category],
                    false,
                )
                .await
            }
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
            Command::Teams(args) => args.into_legacy(),
            Command::Settings(args) => args.into_legacy(),
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

    #[test]
    fn teams_plan_and_apply_map_to_the_access_category_with_a_scope_note() {
        let plan = legacy(&["ward", "teams", "plan"]);
        assert_eq!(plan.category, Category::Access);
        assert_eq!(plan.action, LegacyAction::Plan);
        assert!(plan.note.is_some_and(|note| note.contains("collaborators")));

        let apply = legacy(&["ward", "teams", "apply", "-y"]);
        assert_eq!(
            apply.action,
            LegacyAction::Apply {
                yes: true,
                skip_verify: false
            }
        );
        assert!(apply.note.is_some());
        assert_eq!(
            apply.warning(),
            "warning: 'ward teams apply' is deprecated and will be removed in 0.6.0; use 'ward apply --category access'"
        );
    }

    #[test]
    fn teams_list_maps_to_audit_and_teams_audit_maps_to_drift() {
        let list = legacy(&["ward", "teams", "list"]);
        assert_eq!(list.action, LegacyAction::Audit);
        assert!(list.audit_section().is_some());
        assert_eq!(
            list.warning(),
            "warning: 'ward teams list' is deprecated and will be removed in 0.6.0; use 'ward audit --category access'"
        );

        let audit = legacy(&["ward", "teams", "audit"]);
        assert_eq!(audit.action, LegacyAction::Drift);
        assert_eq!(
            audit.warning(),
            "warning: 'ward teams audit' is deprecated and will be removed in 0.6.0; use 'ward drift --category access'"
        );
    }

    #[test]
    fn settings_maps_to_the_repository_category() {
        let plan = legacy(&["ward", "settings", "plan"]);
        assert_eq!(plan.category, Category::Repository);
        assert_eq!(plan.action, LegacyAction::Plan);
        assert!(plan.note.is_some_and(|note| note.contains("wider")));
        assert_eq!(
            plan.warning(),
            "warning: 'ward settings plan' is deprecated and will be removed in 0.6.0; use 'ward plan --category repository'"
        );

        let audit = legacy(&["ward", "settings", "audit"]);
        assert_eq!(audit.action, LegacyAction::Drift);
        assert_eq!(
            audit.warning(),
            "warning: 'ward settings audit' is deprecated and will be removed in 0.6.0; use 'ward drift --category repository'"
        );
    }

    fn settings_args(args: &[&str]) -> SettingsArgs {
        let Command::Settings(args) = Cli::parse_from(args).command else {
            panic!("expected settings command");
        };
        args
    }

    #[test]
    fn settings_ruleset_option_errors_with_the_manifest_snippet() {
        for action in ["plan", "apply"] {
            let error = settings_args(&["ward", "settings", action, "--ruleset", "copilot-review"])
                .precheck()
                .unwrap_err()
                .to_string();
            assert!(error.contains(COPILOT_REVIEW_SNIPPET), "{error}");
            assert!(error.contains("sensitive = true"), "{error}");
        }
    }

    #[test]
    fn settings_without_the_ruleset_option_passes_the_precheck() {
        settings_args(&["ward", "settings", "plan"])
            .precheck()
            .unwrap();
        settings_args(&["ward", "settings", "audit"])
            .precheck()
            .unwrap();
    }
}
