mod background;
mod event_loop;
mod render;
mod state;

use std::io;

use anyhow::Result;
use crossterm::{
    ExecutableCommand,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;

use crate::config::Manifest;
use crate::github::Client;

use event_loop::run_loop;
use state::App;

pub async fn run(client: &Client, manifest: &Manifest) -> Result<()> {
    let systems: Vec<(String, String)> = manifest
        .systems
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();

    if systems.is_empty() {
        anyhow::bail!("No systems defined in ward.toml - add [[systems]] entries first");
    }

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(systems, manifest.security.checks.clone());

    let result = run_loop(&mut terminal, &mut app, client, manifest).await;

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}
