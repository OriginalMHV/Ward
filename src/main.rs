use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use ward::cli::{Cli, Command};
use ward::config::Manifest;
use ward::github::Client;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    init_tracing(cli.verbose);

    let manifest = Manifest::load(cli.config.as_deref())?;
    let org = cli.org.as_deref().unwrap_or(&manifest.org.name);
    let client = Client::new(org, cli.parallelism).await?;

    match cli.command {
        Command::Repos(cmd) => cmd.run(&client, &manifest, cli.system.as_deref()).await,
        Command::Security(cmd) => {
            cmd.run(&client, &manifest, cli.system.as_deref(), cli.repo.as_deref())
                .await
        }
        Command::Commit(cmd) => {
            cmd.run(&client, &manifest, cli.system.as_deref(), cli.repo.as_deref())
                .await
        }
        Command::Audit(cmd) => cmd.run(&client, &manifest, cli.system.as_deref()).await,
        Command::Init => ward::cli::init::run(),
    }
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
        .with(fmt::layer().with_target(false).without_time())
        .init();
}
