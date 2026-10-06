pub mod apply;
pub mod args;
pub mod audit;
pub mod config_cmd;
pub mod deprecated;
pub mod doctor;
pub mod drift;
pub mod import;
pub mod init;
mod output;
pub mod plan;
pub mod repos;

use clap::Parser;

const AFTER_HELP: &str = "\x1b[1mGetting Started:\x1b[0m
  init, import, doctor, config  Create or import a manifest, check your setup

\x1b[1mPlan & Apply:\x1b[0m
  plan, apply                   Preview and apply changes across categories

\x1b[1mMonitor:\x1b[0m
  drift, audit                  Detect drift, report current state
  repos                         List repositories

\x1b[1mCommon options (after the subcommand):\x1b[0m
  --category C,..               Limit to categories (plan, apply, drift, audit)
  --org, --system, --repo       Narrow the target
  --format text|json            Output format

\x1b[2mNew to Ward? Run: ward import OWNER/REPO → ward plan\x1b[0m
\x1b[2mFull tutorial: https://github.com/OriginalMHV/Ward/blob/main/docs/getting-started.md\x1b[0m";

#[derive(Parser)]
#[command(
    name = "ward",
    about = "GitHub repository management as code. Plan, apply, verify.",
    long_about = "Ward treats GitHub repository management as infrastructure-as-code.\n\
                  Declare your desired state in ward.toml, preview changes with plan,\n\
                  apply them, and verify the result.\n\n\
                  Start here: ward import OWNER/REPO → ward doctor → ward plan",
    version,
    propagate_version = true,
    after_long_help = AFTER_HELP,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Max concurrent operations
    #[arg(long, global = true, default_value_t = 5)]
    pub parallelism: usize,

    /// Path to ward.toml
    #[arg(long, global = true)]
    pub config: Option<String>,

    /// Increase log verbosity (-v, -vv, -vvv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(clap::Subcommand)]
pub enum Command {
    // --- Getting Started ---
    /// Create a minimal ward.toml (use `ward import` to build one from a repository)
    #[command(display_order = 1)]
    Init(init::InitCommand),

    /// Diagnose your Ward setup (token, config, API)
    #[command(display_order = 3)]
    Doctor(doctor::DoctorCommand),

    /// Manage ward.toml configuration
    #[command(display_order = 4)]
    Config(config_cmd::ConfigCommand),

    // --- Inspect ---
    /// List repositories
    #[command(display_order = 10)]
    Repos(repos::ReposCommand),

    // --- Plan & Apply ---
    /// Unified compliance plan across all features (start here)
    #[command(display_order = 20)]
    Plan(plan::PlanCommand),

    /// Apply desired manifest state across categories (plan, apply, verify)
    #[command(display_order = 21)]
    Apply(apply::ApplyCommand),

    /// Deprecated. Use `ward plan|apply|audit --category security`
    #[command(hide = true)]
    Security(deprecated::LegacyArgs),

    /// Deprecated. Use `ward plan|apply|audit --category rulesets`
    #[command(hide = true)]
    Rulesets(deprecated::LegacyArgs),

    /// Deprecated. Use `ward plan|apply --category files`
    #[command(hide = true)]
    Commit(deprecated::LegacyArgs),

    /// Deprecated. Use `ward plan|apply --category access`
    #[command(hide = true)]
    Teams(deprecated::TeamsArgs),

    /// Deprecated. Use `ward plan|apply|audit --category branch-protection`
    #[command(hide = true)]
    Protection(deprecated::LegacyArgs),

    /// Deprecated. Use `ward plan|apply --category repository`
    #[command(hide = true)]
    Settings(deprecated::SettingsArgs),

    // --- Monitor ---
    /// Detect configuration drift from desired state
    #[command(display_order = 40)]
    Drift(drift::DriftCommand),

    /// Full compliance audit across repos
    #[command(display_order = 41)]
    Audit(audit::AuditCommand),

    // --- Advanced ---
    /// Import an existing repository as an exact Ward baseline
    #[command(display_order = 60)]
    Import(import::ImportCommand),

    /// Generate shell completions
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

/// The command tree for shell completions, without hidden commands and flags.
pub fn completion_command() -> clap::Command {
    without_hidden(&<Cli as clap::CommandFactory>::command())
}

fn without_hidden(command: &clap::Command) -> clap::Command {
    let mut visible = clap::Command::new(command.get_name().to_owned())
        .args(
            command
                .get_arguments()
                .filter(|arg| !arg.is_hide_set())
                .cloned(),
        )
        .subcommands(
            command
                .get_subcommands()
                .filter(|sub| !sub.is_hide_set())
                .map(without_hidden),
        );
    if let Some(about) = command.get_about() {
        visible = visible.about(about.clone());
    }
    if let Some(version) = command.get_version() {
        visible = visible.version(version.to_owned());
    }
    visible
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    #[test]
    fn completion_command_leaves_out_hidden_commands_and_flags() {
        let mut command = completion_command();
        command.build();
        let names: Vec<_> = command
            .get_subcommands()
            .map(|sub| sub.get_name())
            .collect();
        assert!(names.contains(&"plan"), "{names:?}");
        for hidden in [
            "security",
            "rulesets",
            "commit",
            "teams",
            "protection",
            "settings",
        ] {
            assert!(!names.contains(&hidden), "{hidden} in {names:?}");
        }
        let plan = command.find_subcommand("plan").unwrap();
        assert!(plan.get_arguments().any(|arg| arg.get_id() == "format"));
        assert!(
            plan.get_arguments()
                .all(|arg| arg.get_long() != Some("json"))
        );
    }
}
