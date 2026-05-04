use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;
use tokio::sync::mpsc;

use crate::cache::{self, DEFAULT_MAX_AGE};
use crate::config::Manifest;
use crate::github::Client;

use super::background::{
    save_to_disk_cache, spawn_custom_checks, spawn_protection_apply, spawn_repo_load,
    spawn_security_apply, spawn_security_load, spawn_settings_apply, spawn_template_deploy,
};
use super::render::draw;
use super::state::{App, BgMessage, PendingAction, RepoEntry, Tab, has_loaded_repo_data};

pub(super) fn refresh_cached_repo_data_if_needed(
    app: &mut App,
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    sys_name: &str,
) {
    let needs_repo_data = app.repos.iter().any(|entry| !has_loaded_repo_data(entry));
    let needs_custom_checks = !app.custom_checks.is_empty()
        && app
            .repos
            .iter()
            .any(|entry| entry.custom_checks.iter().any(Option::is_none));

    if !needs_repo_data && !needs_custom_checks {
        return;
    }

    app.status_msg = format!(
        "{} repos for {sys_name} (cached, refreshing live data)",
        app.repos.len()
    );

    if needs_repo_data {
        spawn_security_load(tx, client, &app.repos);
    }
    if needs_custom_checks {
        spawn_custom_checks(tx, client, &app.repos, &app.custom_checks);
    }
}

pub(super) fn switch_to_cached_or_prompt(
    app: &mut App,
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
) {
    let (sys_id, sys_name) = app.systems[app.selected_system].clone();
    if let Some(cached) = app.cache.get(&sys_id) {
        app.repos = cached.clone();
        app.list_state.select(Some(0));
        app.status_msg = format!("{} repos for {sys_name} (cached)", app.repos.len());
        refresh_cached_repo_data_if_needed(app, tx, client, &sys_name);
    } else if let Some(disk_hit) = app
        .disk_cache
        .as_ref()
        .and_then(|dc| dc.load(&sys_id, DEFAULT_MAX_AGE))
    {
        let age_str = cache::format_age(&disk_hit.cached_at);
        let num_checks = app.custom_checks.len();
        let entries: Vec<RepoEntry> = disk_hit
            .repos
            .into_iter()
            .map(|ce| RepoEntry {
                repo: ce.repo,
                security: ce.security,
                dependency_graph: ce.dependency_graph,
                custom_checks: vec![None; num_checks],
            })
            .collect();
        let count = entries.len();
        app.repos = entries.clone();
        app.cache.insert(sys_id.clone(), entries);
        app.list_state.select(Some(0));
        app.status_msg = format!("{count} repos for {sys_name} (cached, {age_str})");
        refresh_cached_repo_data_if_needed(app, tx, client, &sys_name);
    } else {
        app.repos.clear();
        app.list_state.select(Some(0));
        app.status_msg = format!("{sys_name} ({sys_id}). Press Enter to load.");
    }
}

pub(super) async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &Client,
    manifest: &Manifest,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<BgMessage>();

    loop {
        if app.loading || app.is_loading_security() {
            app.spinner_frame = app.spinner_frame.wrapping_add(1);
        }

        terminal.draw(|f| draw(f, app))?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                BgMessage::ReposLoaded(entries) => {
                    let count = entries.len();
                    let (sys_id, sys_name) = &app.systems[app.selected_system];
                    app.status_msg =
                        format!("Loaded {count} repos for {sys_name}. Fetching security + SBOM...");
                    app.repos = entries;
                    app.list_state.select(Some(0));
                    app.loading = false;
                    app.cache.insert(sys_id.clone(), app.repos.clone());
                    spawn_security_load(&tx, client, &app.repos);
                    spawn_custom_checks(&tx, client, &app.repos, &app.custom_checks);
                }
                BgMessage::SecurityLoaded(idx, state, dependency_graph) => {
                    if let Some(entry) = app.repos.get_mut(idx) {
                        entry.security = Some(state);
                        entry.dependency_graph = Some(dependency_graph);
                    }
                    let loaded = app.repos.iter().filter(|r| has_loaded_repo_data(r)).count();
                    let total = app.repos.len();
                    if loaded == total
                        && app
                            .repos
                            .iter()
                            .all(|r| r.custom_checks.iter().all(Option::is_some))
                    {
                        let (sys_id, sys_name) = &app.systems[app.selected_system];
                        app.status_msg = format!("{total} repos loaded for {sys_name}");
                        app.cache.insert(sys_id.clone(), app.repos.clone());
                        save_to_disk_cache(&app.disk_cache, sys_id, &app.repos);
                    } else {
                        app.status_msg = format!(
                            "[{}] Security + SBOM: {loaded}/{total}...",
                            app.spinner_char()
                        );
                    }
                }
                BgMessage::CustomCheckLoaded(repo_idx, check_idx, result) => {
                    if let Some(entry) = app.repos.get_mut(repo_idx) {
                        if let Some(slot) = entry.custom_checks.get_mut(check_idx) {
                            *slot = Some(result);
                        }
                    }
                    let all_security = app.repos.iter().all(has_loaded_repo_data);
                    let all_custom = app
                        .repos
                        .iter()
                        .all(|r| r.custom_checks.iter().all(Option::is_some));
                    if all_security && all_custom {
                        let total = app.repos.len();
                        let (sys_id, sys_name) = &app.systems[app.selected_system];
                        app.status_msg = format!("{total} repos loaded for {sys_name}");
                        app.cache.insert(sys_id.clone(), app.repos.clone());
                        save_to_disk_cache(&app.disk_cache, sys_id, &app.repos);
                    }
                }
                BgMessage::SecurityApplied(repo, result) => {
                    let spinner = app.spinner_char();
                    match result {
                        Ok(()) => {
                            app.status_msg = format!("Applied security to {repo}");
                            if let Some((ref mut done, total)) = app.bulk_progress {
                                *done += 1;
                                if *done >= total {
                                    app.status_msg =
                                        format!("Bulk security complete: {total}/{total}");
                                    app.bulk_progress = None;
                                    app.loading = false;
                                } else {
                                    app.status_msg =
                                        format!("[{spinner}] Bulk security: {done}/{total}...");
                                }
                            } else {
                                app.loading = false;
                            }
                        }
                        Err(e) => {
                            app.status_msg = format!("Failed to apply to {repo}: {e}");
                            if let Some((ref mut done, total)) = app.bulk_progress {
                                *done += 1;
                                if *done >= total {
                                    app.status_msg =
                                        format!("Bulk security done ({total}), last error: {e}");
                                    app.bulk_progress = None;
                                    app.loading = false;
                                }
                            } else {
                                app.loading = false;
                            }
                        }
                    }
                }
                BgMessage::ProtectionApplied(repo, result) => {
                    match result {
                        Ok(()) => {
                            app.status_msg = format!("Applied branch protection to {repo}");
                        }
                        Err(e) => {
                            app.status_msg = format!("Failed branch protection on {repo}: {e}");
                        }
                    }
                    app.loading = false;
                }
                BgMessage::TemplateDeployed(repo, template, result) => {
                    match result {
                        Ok(pr_url) => {
                            app.status_msg = format!("Deployed {template} to {repo}: {pr_url}");
                        }
                        Err(e) => {
                            app.status_msg = format!("Template {template} failed on {repo}: {e}");
                        }
                    }
                    app.loading = false;
                }
                BgMessage::SettingsApplied(repo, result) => {
                    match result {
                        Ok(detail) => {
                            app.status_msg = format!("Settings applied to {repo}: {detail}");
                        }
                        Err(e) => {
                            app.status_msg = format!("Settings failed on {repo}: {e}");
                        }
                    }
                    app.loading = false;
                }
                BgMessage::Error(e) => {
                    app.status_msg = format!("Error: {e}");
                    app.loading = false;
                }
            }
        }

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if app.pending_confirm.is_some() {
                match key.code {
                    KeyCode::Char('y') => {
                        let action = app.pending_confirm.take();
                        match action {
                            Some(PendingAction::ApplySecurity(repo)) => {
                                let sys_id = &app.systems[app.selected_system].0;
                                let sec_config = manifest.security_for_system(sys_id).clone();
                                app.loading = true;
                                app.status_msg = format!(
                                    "[{}] Applying security to {repo}...",
                                    app.spinner_char()
                                );
                                spawn_security_apply(&tx, client, &repo, &sec_config);
                            }
                            Some(PendingAction::ApplyProtection(repo)) => {
                                if let Some(entry) = app.repos.iter().find(|e| e.repo.name == repo)
                                {
                                    let branch = entry.repo.default_branch.clone();
                                    let cfg = manifest.branch_protection.clone();
                                    app.loading = true;
                                    app.status_msg = format!(
                                        "[{}] Applying protection to {repo}...",
                                        app.spinner_char()
                                    );
                                    spawn_protection_apply(&tx, client, &repo, &branch, &cfg);
                                }
                            }
                            Some(PendingAction::DeployTemplate(repo, template)) => {
                                if let Some(entry) = app.repos.iter().find(|e| e.repo.name == repo)
                                {
                                    let default_br = entry.repo.default_branch.clone();
                                    app.loading = true;
                                    app.status_msg = format!(
                                        "[{}] Deploying {template} to {repo}...",
                                        app.spinner_char()
                                    );
                                    spawn_template_deploy(
                                        &tx,
                                        client,
                                        manifest,
                                        &repo,
                                        &default_br,
                                        &template,
                                    );
                                }
                            }
                            Some(PendingAction::ApplySettings(repo)) => {
                                if let Some(entry) = app.repos.iter().find(|e| e.repo.name == repo)
                                {
                                    let default_br = entry.repo.default_branch.clone();
                                    app.loading = true;
                                    app.status_msg = format!(
                                        "[{}] Applying settings to {repo}...",
                                        app.spinner_char()
                                    );
                                    spawn_settings_apply(&tx, client, manifest, &repo, &default_br);
                                }
                            }
                            Some(PendingAction::BulkApplySecurity) => {
                                let filtered: Vec<String> = app
                                    .filtered_repos()
                                    .iter()
                                    .map(|e| e.repo.name.clone())
                                    .collect();
                                let total = filtered.len();
                                let sys_id = &app.systems[app.selected_system].0;
                                let sec_config = manifest.security_for_system(sys_id).clone();
                                app.loading = true;
                                app.bulk_progress = Some((0, total));
                                app.status_msg =
                                    format!("[{}] Bulk security: 0/{total}...", app.spinner_char());
                                for repo in &filtered {
                                    spawn_security_apply(&tx, client, repo, &sec_config);
                                }
                            }
                            Some(PendingAction::SelectTemplate(_)) | None => {}
                        }
                    }
                    KeyCode::Char('d') => {
                        if let Some(PendingAction::SelectTemplate(repo)) =
                            app.pending_confirm.take()
                        {
                            app.pending_confirm = Some(PendingAction::DeployTemplate(
                                repo.clone(),
                                "dependabot".to_owned(),
                            ));
                            app.status_msg = format!("Deploy dependabot to {repo}? (y/n)");
                        }
                    }
                    KeyCode::Char('c') => {
                        if let Some(PendingAction::SelectTemplate(repo)) =
                            app.pending_confirm.take()
                        {
                            app.pending_confirm = Some(PendingAction::DeployTemplate(
                                repo.clone(),
                                "codeql".to_owned(),
                            ));
                            app.status_msg = format!("Deploy codeql to {repo}? (y/n)");
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(PendingAction::SelectTemplate(repo)) =
                            app.pending_confirm.take()
                        {
                            app.pending_confirm = Some(PendingAction::DeployTemplate(
                                repo.clone(),
                                "dependency-submission".to_owned(),
                            ));
                            app.status_msg =
                                format!("Deploy dependency-submission to {repo}? (y/n)");
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        app.pending_confirm = None;
                        app.status_msg = "Cancelled.".to_owned();
                    }
                    _ => {}
                }
                continue;
            }

            if app.is_filtering {
                match key.code {
                    KeyCode::Esc => {
                        app.is_filtering = false;
                        app.filter.clear();
                    }
                    KeyCode::Enter => {
                        app.is_filtering = false;
                    }
                    KeyCode::Backspace => {
                        app.filter.pop();
                    }
                    KeyCode::Char(c) => {
                        app.filter.push(c);
                        app.list_state.select(Some(0));
                    }
                    _ => {}
                }
                continue;
            }

            match key.code {
                KeyCode::Char('q') => app.should_quit = true,
                KeyCode::Char('1') => app.tab = Tab::Repos,
                KeyCode::Char('2') => app.tab = Tab::Security,
                KeyCode::Char('3') => app.tab = Tab::Actions,
                KeyCode::Char('?') => app.tab = Tab::Help,
                KeyCode::Char('/') => {
                    app.is_filtering = true;
                    app.filter.clear();
                }
                KeyCode::Char('a') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::ApplySecurity(repo.clone()));
                        app.status_msg = format!("Apply security to {repo}? (y/n)");
                    }
                }
                KeyCode::Char('p') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::ApplyProtection(repo.clone()));
                        app.status_msg = format!("Apply branch protection to {repo}? (y/n)");
                    }
                }
                KeyCode::Char('t') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::SelectTemplate(repo));
                        app.status_msg =
                            "Deploy template: (d)ependabot (c)odeql (s)ubmission".to_owned();
                    }
                }
                KeyCode::Char('S') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::ApplySettings(repo.clone()));
                        app.status_msg = format!(
                            "Apply settings (copilot ruleset + instructions) to {repo}? (y/n)"
                        );
                    }
                }
                KeyCode::Char('A') => {
                    let count = app.filtered_repos().len();
                    if count > 0 {
                        app.pending_confirm = Some(PendingAction::BulkApplySecurity);
                        app.status_msg =
                            format!("Apply security to all {count} filtered repos? (y/n)");
                    }
                }
                KeyCode::Char('r') if !app.loading => {
                    app.loading = true;
                    let sys_id = &app.systems[app.selected_system].0;
                    app.status_msg = format!(
                        "[{}] Loading {}...",
                        app.spinner_char(),
                        app.systems[app.selected_system].1
                    );
                    spawn_repo_load(&tx, client, manifest, sys_id, app.custom_checks.len());
                }
                KeyCode::Char('R') if !app.loading => {
                    let sys_id = app.systems[app.selected_system].0.clone();
                    app.cache.remove(&sys_id);
                    cache::try_invalidate(&app.disk_cache, &sys_id);
                    app.loading = true;
                    app.status_msg = format!(
                        "[{}] Force loading {}...",
                        app.spinner_char(),
                        app.systems[app.selected_system].1
                    );
                    spawn_repo_load(&tx, client, manifest, &sys_id, app.custom_checks.len());
                }
                KeyCode::Char('l') | KeyCode::Enter if !app.loading => {
                    app.loading = true;
                    let sys_id = &app.systems[app.selected_system].0;
                    app.status_msg = format!(
                        "[{}] Loading {}...",
                        app.spinner_char(),
                        app.systems[app.selected_system].1
                    );
                    spawn_repo_load(&tx, client, manifest, sys_id, app.custom_checks.len());
                }
                KeyCode::Tab | KeyCode::Char('s') if !app.systems.is_empty() => {
                    app.selected_system = (app.selected_system + 1) % app.systems.len();
                    switch_to_cached_or_prompt(app, &tx, client);
                }
                KeyCode::BackTab if !app.systems.is_empty() => {
                    app.selected_system = if app.selected_system == 0 {
                        app.systems.len() - 1
                    } else {
                        app.selected_system - 1
                    };
                    switch_to_cached_or_prompt(app, &tx, client);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let len = app.filtered_repos().len();
                    if len > 0 {
                        let i = app.list_state.selected().unwrap_or(0);
                        app.list_state.select(Some((i + 1).min(len - 1)));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let i = app.list_state.selected().unwrap_or(0);
                    app.list_state.select(Some(i.saturating_sub(1)));
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
