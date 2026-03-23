pub mod audit;
pub mod commit;
pub mod config_cmd;
pub mod drift;
pub mod import;
pub mod init;
pub mod plan;
pub mod policy;
pub mod protection;
pub mod repos;
pub mod rollback;
pub mod rulesets;
pub mod security;
pub mod settings;
pub mod teams;
pub mod template_cmd;
pub mod tui;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "ward",
    about = "GitHub repository management for developers. Plan, apply, verify.",
    version,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// GitHub organization (overrides ward.toml)
    #[arg(long, global = true)]
    pub org: Option<String>,

    /// Filter to a specific system (e.g., backend)
    #[arg(long, global = true)]
    pub system: Option<String>,

    /// Target a single repository
    #[arg(long, global = true)]
    pub repo: Option<String>,

    /// Output as JSON
    #[arg(long, global = true, default_value_t = false)]
    pub json: bool,

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
    /// List and inspect repositories
    Repos(repos::ReposCommand),

    /// Manage security features (Dependabot, secret scanning, CodeQL)
    Security(security::SecurityCommand),

    /// Manage repository settings and rulesets
    Settings(settings::SettingsCommand),

    /// Commit files/templates to repositories
    Commit(commit::CommitCommand),

    /// Manage branch protection rules
    Protection(protection::ProtectionCommand),

    /// Detect configuration drift from desired state
    Drift(drift::DriftCommand),

    /// Manage repository rulesets
    Rulesets(rulesets::RulesetsCommand),

    /// Manage team access to repositories
    Teams(teams::TeamsCommand),

    /// Rollback changes using the audit log
    Rollback(rollback::RollbackCommand),

    /// Full compliance audit across repos
    Audit(audit::AuditCommand),

    /// Import existing org state into ward.toml
    Import(import::ImportCommand),

    /// Unified plan across all checks
    Plan(plan::PlanCommand),

    /// Check repos against policy rules
    Policy(policy::PolicyCommand),

    /// Launch interactive terminal UI
    Tui,

    /// Interactive setup wizard (creates ward.toml)
    Init(init::InitCommand),

    /// Manage ward.toml configuration
    Config(config_cmd::ConfigCommand),

    /// Manage workflow templates
    Template(template_cmd::TemplateCommand),

    /// Generate shell completions
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}
