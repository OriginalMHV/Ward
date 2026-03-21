use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::CrosstermBackend,
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};
use tokio::sync::mpsc;

use crate::config::{Manifest, SecurityConfig};
use crate::github::Client;
use crate::github::repos::Repository;
use crate::github::security::SecurityState;

const SPINNER_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

enum Tab {
    Repos,
    Security,
    Actions,
    Help,
}

enum BgMessage {
    ReposLoaded(Vec<RepoEntry>),
    SecurityLoaded(usize, SecurityState),
    SecurityApplied(String, std::result::Result<(), String>),
    Error(String),
}

enum PendingAction {
    ApplySecurity(String),
}

struct App {
    tab: Tab,
    repos: Vec<RepoEntry>,
    list_state: ListState,
    filter: String,
    is_filtering: bool,
    should_quit: bool,
    status_msg: String,
    systems: Vec<(String, String)>,
    selected_system: usize,
    loading: bool,
    spinner_frame: usize,
    pending_confirm: Option<PendingAction>,
    cache: std::collections::HashMap<String, Vec<RepoEntry>>,
}

#[derive(Clone)]
struct RepoEntry {
    repo: Repository,
    security: Option<SecurityState>,
}

impl App {
    fn new(systems: Vec<(String, String)>) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            tab: Tab::Repos,
            repos: Vec::new(),
            list_state: state,
            filter: String::new(),
            is_filtering: false,
            should_quit: false,
            status_msg: "Press Enter to load repos".to_owned(),
            systems,
            selected_system: 0,
            loading: false,
            spinner_frame: 0,
            pending_confirm: None,
            cache: std::collections::HashMap::new(),
        }
    }

    fn filtered_repos(&self) -> Vec<&RepoEntry> {
        if self.filter.is_empty() {
            return self.repos.iter().collect();
        }

        let terms: Vec<&str> = self.filter.split_whitespace().collect();
        let includes: Vec<&str> = terms
            .iter()
            .filter(|t| !t.starts_with('!'))
            .copied()
            .collect();
        let excludes: Vec<String> = terms
            .iter()
            .filter(|t| t.starts_with('!'))
            .map(|t| t[1..].to_lowercase())
            .collect();

        self.repos
            .iter()
            .filter(|r| {
                let name = r.repo.name.to_lowercase();
                let include_ok = includes.is_empty()
                    || includes
                        .iter()
                        .any(|inc| name.contains(&inc.to_lowercase()));
                let exclude_ok = excludes.iter().all(|exc| !name.contains(exc.as_str()));
                include_ok && exclude_ok
            })
            .collect()
    }

    fn selected_repo(&self) -> Option<&RepoEntry> {
        let filtered = self.filtered_repos();
        self.list_state
            .selected()
            .and_then(|i| filtered.get(i).copied())
    }

    fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    fn is_loading_security(&self) -> bool {
        !self.repos.is_empty() && self.repos.iter().any(|r| r.security.is_none())
    }
}

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
    let mut app = App::new(systems);

    let result = run_loop(&mut terminal, &mut app, client, manifest).await;

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn spawn_repo_load(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    manifest: &Manifest,
    system_id: &str,
) {
    let tx = tx.clone();
    let org = client.org.clone();
    let http = client.http.clone();
    let base_url = client.base_url.clone();
    let semaphore = client.semaphore.clone();
    let excludes = manifest.exclude_patterns_for_system(system_id);
    let sys_id = system_id.to_owned();

    tokio::spawn(async move {
        let bg_client = Client {
            http,
            org,
            semaphore,
            base_url,
        };

        match bg_client.list_repos_for_system(&sys_id, &excludes).await {
            Ok(repos) => {
                let entries: Vec<RepoEntry> = repos
                    .into_iter()
                    .map(|repo| RepoEntry {
                        repo,
                        security: None,
                    })
                    .collect();
                let _ = tx.send(BgMessage::ReposLoaded(entries));
            }
            Err(e) => {
                let _ = tx.send(BgMessage::Error(e.to_string()));
            }
        }
    });
}

fn spawn_security_load(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repos: &[RepoEntry],
) {
    for (idx, entry) in repos.iter().enumerate() {
        let tx = tx.clone();
        let http = client.http.clone();
        let org = client.org.clone();
        let base_url = client.base_url.clone();
        let semaphore = client.semaphore.clone();
        let repo_name = entry.repo.name.clone();

        tokio::spawn(async move {
            let bg_client = Client {
                http,
                org,
                semaphore,
                base_url,
            };

            if let Ok(state) = bg_client.get_security_state(&repo_name).await {
                let _ = tx.send(BgMessage::SecurityLoaded(idx, state));
            }
        });
    }
}

fn spawn_security_apply(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repo_name: &str,
    config: &SecurityConfig,
) {
    let tx = tx.clone();
    let http = client.http.clone();
    let org = client.org.clone();
    let base_url = client.base_url.clone();
    let semaphore = client.semaphore.clone();
    let repo = repo_name.to_owned();
    let cfg = config.clone();

    tokio::spawn(async move {
        let bg_client = Client {
            http,
            org,
            semaphore,
            base_url,
        };

        let result = async {
            if cfg.dependabot_alerts {
                bg_client.enable_dependabot_alerts(&repo).await?;
            }
            if cfg.dependabot_security_updates {
                bg_client.enable_dependabot_security_updates(&repo).await?;
            }
            bg_client
                .set_security_features(
                    &repo,
                    cfg.secret_scanning,
                    cfg.secret_scanning_ai_detection,
                    cfg.push_protection,
                )
                .await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let msg = match result {
            Ok(()) => BgMessage::SecurityApplied(repo, Ok(())),
            Err(e) => BgMessage::SecurityApplied(repo, Err(e.to_string())),
        };
        let _ = tx.send(msg);
    });
}

fn switch_to_cached_or_prompt(app: &mut App) {
    let (sys_id, sys_name) = &app.systems[app.selected_system];
    if let Some(cached) = app.cache.get(sys_id) {
        app.repos = cached.clone();
        app.list_state.select(Some(0));
        app.status_msg = format!("{} repos for {sys_name} (cached)", app.repos.len());
    } else {
        app.repos.clear();
        app.list_state.select(Some(0));
        app.status_msg = format!("{sys_name} ({sys_id}). Press Enter to load.");
    }
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &Client,
    manifest: &Manifest,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<BgMessage>();

    loop {
        // Advance spinner each render cycle when loading
        if app.loading || app.is_loading_security() {
            app.spinner_frame = app.spinner_frame.wrapping_add(1);
        }

        terminal.draw(|f| draw(f, app))?;

        // Check for background task results (non-blocking)
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BgMessage::ReposLoaded(entries) => {
                    let count = entries.len();
                    let (sys_id, sys_name) = &app.systems[app.selected_system];
                    app.status_msg =
                        format!("Loaded {count} repos for {sys_name}. Fetching security...");
                    app.repos = entries;
                    app.list_state.select(Some(0));
                    app.loading = false;
                    app.cache.insert(sys_id.clone(), app.repos.clone());
                    spawn_security_load(&tx, client, &app.repos);
                }
                BgMessage::SecurityLoaded(idx, state) => {
                    if let Some(entry) = app.repos.get_mut(idx) {
                        entry.security = Some(state);
                    }
                    let loaded = app.repos.iter().filter(|r| r.security.is_some()).count();
                    let total = app.repos.len();
                    if loaded == total {
                        let (sys_id, sys_name) = &app.systems[app.selected_system];
                        app.status_msg = format!("{total} repos loaded for {sys_name}");
                        app.cache.insert(sys_id.clone(), app.repos.clone());
                    } else {
                        app.status_msg =
                            format!("[{}] Security: {loaded}/{total}...", app.spinner_char());
                    }
                }
                BgMessage::SecurityApplied(repo, result) => {
                    match result {
                        Ok(()) => {
                            app.status_msg = format!("Applied security to {repo}");
                        }
                        Err(e) => {
                            app.status_msg = format!("Failed to apply to {repo}: {e}");
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

            // Confirmation mode: only y/n accepted
            if app.pending_confirm.is_some() {
                match key.code {
                    KeyCode::Char('y') => {
                        if let Some(PendingAction::ApplySecurity(repo)) = app.pending_confirm.take()
                        {
                            let sys_id = &app.systems[app.selected_system].0;
                            let sec_config = manifest.security_for_system(sys_id).clone();
                            app.loading = true;
                            app.status_msg =
                                format!("[{}] Applying security to {repo}...", app.spinner_char());
                            spawn_security_apply(&tx, client, &repo, &sec_config);
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
                KeyCode::Char('r') => {
                    if !app.loading {
                        app.loading = true;
                        let sys_id = &app.systems[app.selected_system].0;
                        app.status_msg = format!(
                            "[{}] Loading {}...",
                            app.spinner_char(),
                            app.systems[app.selected_system].1
                        );
                        spawn_repo_load(&tx, client, manifest, sys_id);
                    }
                }
                KeyCode::Char('R') => {
                    if !app.loading {
                        let sys_id = app.systems[app.selected_system].0.clone();
                        app.cache.remove(&sys_id);
                        app.loading = true;
                        app.status_msg = format!(
                            "[{}] Force loading {}...",
                            app.spinner_char(),
                            app.systems[app.selected_system].1
                        );
                        spawn_repo_load(&tx, client, manifest, &sys_id);
                    }
                }
                KeyCode::Char('l') | KeyCode::Enter => {
                    if !app.loading {
                        app.loading = true;
                        let sys_id = &app.systems[app.selected_system].0;
                        app.status_msg = format!(
                            "[{}] Loading {}...",
                            app.spinner_char(),
                            app.systems[app.selected_system].1
                        );
                        spawn_repo_load(&tx, client, manifest, sys_id);
                    }
                }
                KeyCode::Tab | KeyCode::Char('s') => {
                    if !app.systems.is_empty() {
                        app.selected_system = (app.selected_system + 1) % app.systems.len();
                        switch_to_cached_or_prompt(app);
                    }
                }
                KeyCode::BackTab => {
                    if !app.systems.is_empty() {
                        app.selected_system = if app.selected_system == 0 {
                            app.systems.len() - 1
                        } else {
                            app.selected_system - 1
                        };
                        switch_to_cached_or_prompt(app);
                    }
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

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);

    match app.tab {
        Tab::Repos => draw_repos_tab(f, chunks[1], app),
        Tab::Security => draw_security_tab(f, chunks[1], app),
        Tab::Actions => draw_actions_tab(f, chunks[1]),
        Tab::Help => draw_help_tab(f, chunks[1]),
    }

    draw_status(f, chunks[2], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let titles = vec!["[1] Repos", "[2] Security", "[3] Actions", "[?] Help"];
    let selected = match app.tab {
        Tab::Repos => 0,
        Tab::Security => 1,
        Tab::Actions => 2,
        Tab::Help => 3,
    };

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .title(" Ward: GitHub Repository Management "),
        )
        .select(selected)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(Style::default().fg(Color::Cyan).bold());

    f.render_widget(tabs, area);
}

fn draw_repos_tab(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let filtered = app.filtered_repos();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|entry| {
            let lang = entry.repo.language.as_deref().unwrap_or("-");
            let (indicator, color) = entry
                .security
                .as_ref()
                .map(|s| {
                    if s.dependabot_alerts && s.secret_scanning && s.push_protection {
                        ("[ok]", Color::Green)
                    } else {
                        ("[!!]", Color::Yellow)
                    }
                })
                .unwrap_or(("[..]", Color::DarkGray));

            ListItem::new(Line::from(vec![
                Span::styled(indicator, Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(entry.repo.name.clone(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(lang, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let title = if app.loading {
        format!(" Repos [{}] loading... ", app.spinner_char())
    } else if app.is_filtering {
        format!(" Repos (filter: {}_) ", app.filter)
    } else if app.filter.is_empty() {
        format!(" Repos ({}) ", filtered.len())
    } else {
        format!(" Repos ({}/{}) ", filtered.len(), app.repos.len())
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, chunks[0], &mut app.list_state.clone());

    let detail = if let Some(entry) = app.selected_repo() {
        let sec = entry.security.as_ref();
        let icon = |b: bool| -> (&str, Color) {
            if b {
                ("[Y]", Color::Green)
            } else {
                ("[N]", Color::Red)
            }
        };

        let mut lines = vec![
            Line::from(Span::styled(
                entry.repo.name.clone(),
                Style::default().fg(Color::Cyan).bold(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::raw("  Language:   "),
                Span::styled(
                    entry.repo.language.as_deref().unwrap_or("-"),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::raw("  Branch:     "),
                Span::raw(&entry.repo.default_branch),
            ]),
            Line::from(vec![
                Span::raw("  Visibility: "),
                Span::raw(&entry.repo.visibility),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Security",
                Style::default().fg(Color::Cyan).bold(),
            )),
        ];

        if let Some(s) = sec {
            let features = [
                ("Dependabot Alerts", s.dependabot_alerts),
                ("Security Updates", s.dependabot_security_updates),
                ("Secret Scanning", s.secret_scanning),
                ("AI Detection", s.secret_scanning_ai_detection),
                ("Push Protection", s.push_protection),
            ];
            for (label, enabled) in features {
                let (text, color) = icon(enabled);
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(text, Style::default().fg(color)),
                    Span::raw(format!(" {label}")),
                ]));
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  [..] Loading...",
                Style::default().fg(Color::DarkGray),
            )));
        }

        lines
    } else {
        vec![Line::from("  Select a repository")]
    };

    let detail_widget = Paragraph::new(detail)
        .block(Block::default().borders(Borders::ALL).title(" Details "))
        .wrap(Wrap { trim: false });

    f.render_widget(detail_widget, chunks[1]);
}

fn draw_security_tab(f: &mut Frame, area: Rect, app: &App) {
    if app.repos.is_empty() {
        let msg = Paragraph::new("  Press Enter to load repos, then switch to this tab.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Security Overview "),
        );
        f.render_widget(msg, area);
        return;
    }

    let col_repo = 37;
    let col_feat = 8;

    let header_line = Line::from(vec![
        Span::styled(
            format!("  {:<col_repo$}", "Repository"),
            Style::default().fg(Color::Cyan).bold(),
        ),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<col_feat$}", "Alerts"),
            Style::default().fg(Color::Cyan).bold(),
        ),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<col_feat$}", "Secret"),
            Style::default().fg(Color::Cyan).bold(),
        ),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<col_feat$}", "AI Det"),
            Style::default().fg(Color::Cyan).bold(),
        ),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<col_feat$}", "Push P"),
            Style::default().fg(Color::Cyan).bold(),
        ),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<col_feat$}", "Sec Up"),
            Style::default().fg(Color::Cyan).bold(),
        ),
    ]);

    let separator = format!(
        "  {:-<col_repo$}-+-{:-<col_feat$}-+-{:-<col_feat$}-+-{:-<col_feat$}-+-{:-<col_feat$}-+-{:-<col_feat$}",
        "", "", "", "", "", ""
    );

    let mut lines = vec![
        header_line,
        Line::from(Span::styled(
            separator,
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let mut secured = 0;
    let mut issues = 0;
    let mut pending = 0;

    for (row_idx, entry) in app.repos.iter().enumerate() {
        let row_bg = if row_idx % 2 == 1 {
            Color::Rgb(30, 30, 40)
        } else {
            Color::Reset
        };
        let row_style = Style::default().bg(row_bg);

        if let Some(s) = entry.security.as_ref() {
            let all_ok = s.dependabot_alerts
                && s.secret_scanning
                && s.secret_scanning_ai_detection
                && s.push_protection;

            if all_ok {
                secured += 1;
            } else {
                issues += 1;
            }

            let icon = |b: bool| -> (&str, Color) {
                if b {
                    ("[Y]", Color::Green)
                } else {
                    ("[N]", Color::Red)
                }
            };

            let features = [
                s.dependabot_alerts,
                s.secret_scanning,
                s.secret_scanning_ai_detection,
                s.push_protection,
                s.dependabot_security_updates,
            ];

            let mut spans = vec![
                Span::styled(format!("  {:<col_repo$}", entry.repo.name), row_style),
                Span::styled("| ", row_style.fg(Color::DarkGray)),
            ];

            for (fi, &feat) in features.iter().enumerate() {
                let (text, color) = icon(feat);
                spans.push(Span::styled(format!(" {text:<6} "), row_style.fg(color)));
                if fi < features.len() - 1 {
                    spans.push(Span::styled("| ", row_style.fg(Color::DarkGray)));
                }
            }

            lines.push(Line::from(spans));
        } else {
            pending += 1;
            lines.push(Line::from(Span::styled(
                format!("  {:<col_repo$}  ...loading", entry.repo.name),
                row_style.fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));
    let summary = if pending > 0 {
        format!("  {secured} secured, {issues} issues, {pending} loading...")
    } else {
        format!("  {secured} secured, {issues} need attention")
    };
    lines.push(Line::from(Span::styled(
        summary,
        Style::default()
            .fg(if issues > 0 {
                Color::Yellow
            } else {
                Color::Green
            })
            .bold(),
    )));

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Security Overview "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

fn draw_actions_tab(f: &mut Frame, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let key_style = Style::default().fg(Color::Yellow).bold();
    let heading = Style::default().fg(Color::Cyan).bold();

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Available Actions (on selected repo)",
            heading,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  a", key_style),
            Span::raw("    Apply security settings to selected repo"),
        ]),
        Line::from(vec![
            Span::styled("  r", key_style),
            Span::raw("    Refresh/reload current system"),
        ]),
        Line::from(vec![
            Span::styled("  R", key_style),
            Span::raw("    Force reload (ignore cache)"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Planned (not yet implemented):", dim)),
        Line::from(Span::styled("  - Branch protection apply", dim)),
        Line::from(Span::styled("  - Template deployment", dim)),
        Line::from(Span::styled("  - Settings management", dim)),
    ];

    let widget =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Actions "));

    f.render_widget(widget, area);
}

fn draw_help_tab(f: &mut Frame, area: Rect) {
    let heading = Style::default().fg(Color::Cyan).bold();
    let dim = Style::default().fg(Color::DarkGray);
    let key = Style::default().fg(Color::Yellow);

    let sep = "─".repeat(15);

    let help = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Navigation", heading),
            Span::raw("                        "),
            Span::styled("Actions", heading),
        ]),
        Line::from(vec![
            Span::styled(format!("  {sep}"), dim),
            Span::raw("                        "),
            Span::styled("─".repeat(7), dim),
        ]),
        Line::from(vec![
            Span::styled("  j/k", key),
            Span::raw(" or arrows  Move up/down  "),
            Span::styled("a", key),
            Span::raw("          Apply security"),
        ]),
        Line::from(vec![
            Span::styled("  Tab/s", key),
            Span::raw("          Next system      "),
            Span::styled("r", key),
            Span::raw("          Reload system"),
        ]),
        Line::from(vec![
            Span::styled("  Shift+Tab", key),
            Span::raw("      Previous system  "),
            Span::styled("R", key),
            Span::raw("          Force reload"),
        ]),
        Line::from(vec![
            Span::styled("  Enter/l", key),
            Span::raw("        Load repos"),
        ]),
        Line::from(vec![
            Span::styled("  /", key),
            Span::raw("              Filter (! to exclude)"),
        ]),
        Line::from(vec![
            Span::styled("  Esc", key),
            Span::raw("            Clear filter         "),
            Span::styled("Tabs", heading),
        ]),
        Line::from(vec![
            Span::raw("                                    "),
            Span::styled("─".repeat(4), dim),
        ]),
        Line::from(vec![
            Span::styled("  General", heading),
            Span::raw("                           "),
            Span::styled("1", key),
            Span::raw("   Repos"),
        ]),
        Line::from(vec![
            Span::styled(format!("  {}", "─".repeat(9)), dim),
            Span::raw("                           "),
            Span::styled("2", key),
            Span::raw("   Security"),
        ]),
        Line::from(vec![
            Span::styled("  q", key),
            Span::raw("              Quit                "),
            Span::styled("3", key),
            Span::raw("   Actions"),
        ]),
        Line::from(vec![
            Span::styled("  ?", key),
            Span::raw("              Help"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Filter Syntax", heading)),
        Line::from(Span::styled(format!("  {}", "─".repeat(13)), dim)),
        Line::from(vec![
            Span::styled("  foo", key),
            Span::raw("           Show repos matching \"foo\""),
        ]),
        Line::from(vec![
            Span::styled("  !ops", key),
            Span::raw("          Hide repos matching \"ops\""),
        ]),
        Line::from(vec![
            Span::styled("  foo !ops", key),
            Span::raw("      Show \"foo\", hide \"ops\""),
        ]),
    ];

    let widget = Paragraph::new(help).block(Block::default().borders(Borders::ALL).title(" Help "));

    f.render_widget(widget, area);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let (sys_id, sys_name) = app
        .systems
        .get(app.selected_system)
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .unwrap_or(("?", "none"));

    let sys_idx = format!("[{}/{}]", app.selected_system + 1, app.systems.len());
    let repo_count = if app.repos.is_empty() {
        String::new()
    } else {
        format!("{} repos", app.repos.len())
    };

    let status_detail = if app.loading {
        format!("[{}] {}", app.spinner_char(), app.status_msg)
    } else if app.is_loading_security() {
        let loaded = app.repos.iter().filter(|r| r.security.is_some()).count();
        let total = app.repos.len();
        format!("[{}] Security: {loaded}/{total}...", app.spinner_char())
    } else {
        app.status_msg.clone()
    };

    let mut spans = vec![
        Span::styled(" System: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{sys_name} ({sys_id})"),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(format!(" {sys_idx}"), Style::default().fg(Color::DarkGray)),
    ];

    if !repo_count.is_empty() {
        spans.push(Span::styled(
            format!(" | {repo_count}"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    spans.extend([
        Span::raw(" | "),
        Span::styled(&status_detail, Style::default().fg(Color::DarkGray)),
        Span::raw(" | "),
        Span::styled(
            "q:quit Tab:sys Enter:load /:filter a:apply",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let status = Line::from(spans);
    let widget = Paragraph::new(status).block(Block::default().borders(Borders::TOP));

    f.render_widget(widget, area);
}
