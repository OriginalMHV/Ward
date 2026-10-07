#![forbid(unsafe_code)]

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use clap_complete::generate;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use ward::cli::deprecated::LegacyCategory;
use ward::cli::{Cli, Command};
use ward::config::Manifest;
use ward::config::manifest::{DEFAULT_MANIFEST_PATH, missing_manifest_message};
use ward::github::Client;
use ward::reconcile::unified::Category;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let verbose = cli.verbose > 0;
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", ward::cli::render::render_error(&error, verbose));
            ward::outcome::exit_code(&error)
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    // Completions need no tracing, token or manifest.
    if let Command::Completions { shell } = cli.command {
        let mut cmd = ward::cli::completion_command();
        let name = <Cli as clap::CommandFactory>::command()
            .get_name()
            .to_string();
        generate(shell, &mut cmd, name, &mut std::io::stdout());
        return Ok(());
    }

    init_tracing(cli.verbose);

    let config = cli.config.as_deref();
    let parallelism = cli.parallelism;

    // Commands that need no client run first. Removed and deprecated commands
    // print their hint here, before any token or manifest is needed.
    match cli.command {
        Command::Completions { .. } => Ok(()),
        Command::Init(cmd) => cmd.run(parallelism).await,
        Command::Import(cmd) => cmd.run(parallelism).await,
        Command::Config(cmd) => cmd.run(config),
        Command::Doctor(cmd) => cmd.run(config).await,
        Command::Repos(cmd) => {
            cmd.precheck()?;
            let (client, manifest) = connect(config, cmd.org(), parallelism)?;
            cmd.run(&client, &manifest).await
        }
        Command::Plan(cmd) => {
            let (client, manifest) = connect(config, cmd.target.org.as_deref(), parallelism)?;
            cmd.run(&client, &manifest).await
        }
        Command::Apply(cmd) => {
            let (client, manifest) = connect(config, cmd.target.org.as_deref(), parallelism)?;
            cmd.run(&client, &manifest).await
        }
        Command::Drift(cmd) => {
            cmd.announce();
            let (client, manifest) = connect(config, cmd.target().org.as_deref(), parallelism)?;
            cmd.run(&client, &manifest).await
        }
        Command::Audit(cmd) => {
            let (client, manifest) = connect(config, cmd.target.org.as_deref(), parallelism)?;
            cmd.run(&client, &manifest).await
        }
        Command::Security(args) => {
            run_legacy(
                args.into_legacy("security", Category::Security),
                config,
                parallelism,
            )
            .await
        }
        Command::Rulesets(args) => {
            run_legacy(
                args.into_legacy("rulesets", Category::Rulesets),
                config,
                parallelism,
            )
            .await
        }
        Command::Commit(args) => {
            run_legacy(
                args.into_legacy("commit", Category::Files),
                config,
                parallelism,
            )
            .await
        }
        Command::Protection(args) => {
            run_legacy(
                args.into_legacy("protection", Category::BranchProtection),
                config,
                parallelism,
            )
            .await
        }
        Command::Teams(args) => run_legacy(args.into_legacy(), config, parallelism).await,
        Command::Settings(args) => {
            args.precheck()?;
            run_legacy(args.into_legacy(), config, parallelism).await
        }
    }
}

/// Print the deprecation warning, then run the replacement command.
async fn run_legacy(
    legacy: LegacyCategory,
    config: Option<&str>,
    parallelism: usize,
) -> Result<()> {
    legacy.announce();
    let (client, manifest) = connect(config, legacy.target.org.as_deref(), parallelism)?;
    legacy.run(&client, &manifest).await
}

/// Load the manifest and build the GitHub client for the target organization.
fn connect(
    config: Option<&str>,
    org_override: Option<&str>,
    parallelism: usize,
) -> Result<(Client, Manifest)> {
    let manifest = Manifest::load(config)?;
    let org = org_override.unwrap_or(&manifest.org.name);

    if org.is_empty() {
        let path = config.unwrap_or(DEFAULT_MANIFEST_PATH);
        if std::path::Path::new(path).exists() {
            anyhow::bail!(
                "{path} does not name an organization. Set name under [org] in the file, or pass --org <name>."
            );
        }
        anyhow::bail!(
            "{} Or pass --org <name> to run without a manifest.",
            missing_manifest_message(path)
        );
    }

    let client = Client::new(org, parallelism)?;
    Ok((client, manifest))
}

fn init_tracing(verbose: u8) {
    let filter = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)))
        .with(
            fmt::layer()
                .with_target(false)
                .without_time()
                .with_writer(std::io::stderr),
        )
        .init();
}
