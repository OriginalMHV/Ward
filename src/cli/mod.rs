pub mod apply;
pub mod audit;
pub mod commit;
pub mod config_cmd;
pub mod doctor;
pub mod drift;
pub mod import;
pub mod init;
pub mod plan;
pub mod protection;
pub mod repos;
pub mod rulesets;
pub mod security;
pub mod settings;
pub mod teams;

use clap::Parser;

const AFTER_HELP: &str = "\x1b[1mGetting Started:\x1b[0m
  init, doctor, config          Set up Ward and configure repos

\x1b[1mPlan & Apply:\x1b[0m
  plan, apply                   Preview and apply changes across categories
  security, rulesets, commit    Manage specific features
  teams, protection, settings   Access control & repo settings

\x1b[1mMonitor:\x1b[0m
  drift, audit                  Detect drift, audit compliance

\x1b[1mAdvanced:\x1b[0m
  import                        Import existing repository state

\x1b[2mNew to Ward? Run: ward init --from OWNER/REPO → ward plan\x1b[0m
\x1b[2mFull tutorial: https://github.com/OriginalMHV/Ward/blob/main/docs/getting-started.md\x1b[0m";

#[derive(Parser)]
#[command(
    name = "ward",
    about = "GitHub repository management as code. Plan, apply, verify.",
    long_about = "Ward treats GitHub repository management as infrastructure-as-code.\n\
                  Declare your desired state in ward.toml, preview changes with plan,\n\
                  apply them, and verify the result.\n\n\
                  Start here: ward init → ward doctor → ward plan",
    version,
    propagate_version = true,
    after_long_help = AFTER_HELP,
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
    // --- Getting Started ---
    /// Create ward.toml interactively or from an existing repository
    #[command(display_order = 1)]
    Init(init::InitCommand),

    /// Diagnose your Ward setup (token, config, API)
    #[command(display_order = 3)]
    Doctor(doctor::DoctorCommand),

    /// Manage ward.toml configuration
    #[command(display_order = 4)]
    Config(config_cmd::ConfigCommand),

    // --- Inspect ---
    /// List and inspect repositories
    #[command(display_order = 10)]
    Repos(repos::ReposCommand),

    // --- Plan & Apply ---
    /// Unified compliance plan across all features (start here)
    #[command(display_order = 20)]
    Plan(plan::PlanCommand),

    /// Apply desired manifest state across categories (plan, apply, verify)
    #[command(display_order = 21)]
    Apply(apply::ApplyCommand),

    /// Manage security features (Dependabot, secret scanning, CodeQL)
    #[command(display_order = 22)]
    Security(security::SecurityCommand),

    /// Manage repository rulesets (branch protection successor)
    #[command(display_order = 23)]
    Rulesets(rulesets::RulesetsCommand),

    /// Commit managed files to repositories (no cloning needed)
    #[command(display_order = 24)]
    Commit(commit::CommitCommand),

    /// Manage team access to repositories
    #[command(display_order = 25)]
    Teams(teams::TeamsCommand),

    /// Manage classic branch protection rules
    #[command(display_order = 26)]
    Protection(protection::ProtectionCommand),

    /// Manage repository settings and rulesets
    #[command(display_order = 27)]
    Settings(settings::SettingsCommand),

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
