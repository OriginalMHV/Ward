use anyhow::Result;
use clap::Parser;
use clap_complete::generate;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use ward::cli::{Cli, Command};
use ward::config::Manifest;
use ward::github::Client;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle completions early, before initializing tracing
    if let Command::Completions { shell } = cli.command {
        let mut cmd = <Cli as clap::CommandFactory>::command();
        let name = cmd.get_name().to_string();
        generate(shell, &mut cmd, name, &mut std::io::stdout());
        return Ok(());
    }

    init_tracing(cli.verbose);

    // Init handles its own client creation (manifest may not exist yet)
    if let Command::Init(cmd) = cli.command {
        return cmd.run().await;
    }

    // Setup handles its own client creation (guided flow)
    if let Command::Setup(cmd) = cli.command {
        return cmd.run().await;
    }

    // Import handles its own client creation (it creates the manifest)
    if let Command::Import(cmd) = cli.command {
        return cmd.run().await;
    }

    if let Command::Config(cmd) = cli.command {
        return cmd.run(cli.config.as_deref());
    }

    if let Command::Template(cmd) = cli.command {
        return cmd.run(cli.config.as_deref());
    }

    if let Command::Doctor(cmd) = cli.command {
        return cmd.run(cli.config.as_deref()).await;
    }

    let manifest = Manifest::load(cli.config.as_deref())?;
    let org = cli.org.as_deref().unwrap_or(&manifest.org.name);

    if org.is_empty() {
        anyhow::bail!(
            "No organization configured. Either set [org] name in ward.toml or pass --org <name>."
        );
    }

    let client = Client::new(org, cli.parallelism).await?;

    match cli.command {
        Command::Repos(cmd) => cmd.run(&client, &manifest, cli.system.as_deref()).await,
        Command::Security(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
            )
            .await
        }
        Command::Settings(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
            )
            .await
        }
        Command::Commit(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
            )
            .await
        }
        Command::Protection(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
            )
            .await
        }
        Command::Drift(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
                cli.json,
            )
            .await
        }
        Command::Rulesets(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
            )
            .await
        }
        Command::Teams(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
            )
            .await
        }
        Command::Rollback(cmd) => cmd.run(&client).await,
        Command::Audit(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
            )
            .await
        }
        Command::Plan(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
                cli.json,
            )
            .await
        }
        Command::Apply(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
                cli.json,
            )
            .await
        }
        Command::Policy(cmd) => {
            cmd.run(
                &client,
                &manifest,
                cli.system.as_deref(),
                cli.repo.as_deref(),
                cli.json,
            )
            .await
        }
        Command::Init(_) => unreachable!(),
        Command::Setup(_) => unreachable!(),
        Command::Import(_) => unreachable!(),
        Command::Config(_) => unreachable!(),
        Command::Template(_) => unreachable!(),
        Command::Doctor(_) => unreachable!(),
        Command::Completions { .. } => unreachable!(),
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
