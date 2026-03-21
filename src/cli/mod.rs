pub mod audit;
pub mod commit;
pub mod init;
pub mod repos;
pub mod security;
pub mod settings;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "ward",
    about = "GitHub repository management for developers — plan, apply, verify.",
    version,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// GitHub organization (overrides ward.toml)
    #[arg(long, global = true)]
    pub org: Option<String>,

    /// Filter to a specific system (e.g., sys-core)
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

    /// Full compliance audit across repos
    Audit(audit::AuditCommand),

    /// Create a ward.toml in the current directory
    Init,
}
