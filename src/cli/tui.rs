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

use crate::config::Manifest;
use crate::github::Client;
use crate::github::repos::Repository;
use crate::github::security::SecurityState;

enum Tab {
    Repos,
    Security,
    Help,
}

enum BgMessage {
    ReposLoaded(Vec<RepoEntry>),
    SecurityLoaded(usize, SecurityState),
    Error(String),
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
            cache: std::collections::HashMap::new(),
        }
    }

    fn filtered_repos(&self) -> Vec<&RepoEntry> {
        if self.filter.is_empty() {
            self.repos.iter().collect()
        } else {
            let f = self.filter.to_lowercase();
            self.repos
                .iter()
                .filter(|r| r.repo.name.to_lowercase().contains(&f))
                .collect()
        }
    }

    fn selected_repo(&self) -> Option<&RepoEntry> {
        let filtered = self.filtered_repos();
        self.list_state
            .selected()
            .and_then(|i| filtered.get(i).copied())
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
                        app.status_msg = format!("Security: {loaded}/{total}...");
                    }
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
                KeyCode::Char('?') => app.tab = Tab::Help,
                KeyCode::Char('/') => {
                    app.is_filtering = true;
                    app.filter.clear();
                }
                KeyCode::Char('l') | KeyCode::Enter => {
                    if !app.loading {
                        app.loading = true;
                        let sys_id = &app.systems[app.selected_system].0;
                        app.status_msg =
                            format!("Loading {}...", app.systems[app.selected_system].1);
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
        Tab::Help => draw_help_tab(f, chunks[1]),
    }

    draw_status(f, chunks[2], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let titles = vec!["[1] Repos", "[2] Security", "[?] Help"];
    let selected = match app.tab {
        Tab::Repos => 0,
        Tab::Security => 1,
        Tab::Help => 2,
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
            let sec_icon = entry
                .security
                .as_ref()
                .map(|s| {
                    if s.dependabot_alerts && s.secret_scanning && s.push_protection {
                        "🟢"
                    } else {
                        "🟡"
                    }
                })
                .unwrap_or("⚪");

            ListItem::new(Line::from(vec![
                Span::raw(format!("{sec_icon} ")),
                Span::styled(entry.repo.name.clone(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(lang, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let title = if app.loading {
        " Repos (loading...) ".to_owned()
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
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[0], &mut app.list_state.clone());

    let detail = if let Some(entry) = app.selected_repo() {
        let sec = entry.security.as_ref();
        let icon = |b: bool| if b { "✅" } else { "❌" };

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
            lines.extend([
                Line::from(format!("  {} Dependabot Alerts", icon(s.dependabot_alerts))),
                Line::from(format!(
                    "  {} Security Updates",
                    icon(s.dependabot_security_updates)
                )),
                Line::from(format!("  {} Secret Scanning", icon(s.secret_scanning))),
                Line::from(format!(
                    "  {} AI Detection",
                    icon(s.secret_scanning_ai_detection)
                )),
                Line::from(format!("  {} Push Protection", icon(s.push_protection))),
            ]);
        } else {
            lines.push(Line::from(Span::styled(
                "  Loading...",
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

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "  {:35} {:6} {:6} {:6} {:6} {:6}",
                "Repository", "Dep.A", "SecSc", "AI", "Push", "SecUp"
            ),
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
    ];

    let mut secured = 0;
    let mut issues = 0;
    let mut pending = 0;

    for entry in &app.repos {
        let sec = entry.security.as_ref();
        let icon = |b: bool| if b { " ✅ " } else { " ❌ " };

        if let Some(s) = sec {
            let all_ok = s.dependabot_alerts
                && s.secret_scanning
                && s.secret_scanning_ai_detection
                && s.push_protection;

            if all_ok {
                secured += 1;
            } else {
                issues += 1;
            }

            lines.push(Line::from(format!(
                "  {:35} {} {} {} {} {}",
                entry.repo.name,
                icon(s.dependabot_alerts),
                icon(s.secret_scanning),
                icon(s.secret_scanning_ai_detection),
                icon(s.push_protection),
                icon(s.dependabot_security_updates),
            )));
        } else {
            pending += 1;
            lines.push(Line::from(Span::styled(
                format!("  {:35}  ...loading", entry.repo.name),
                Style::default().fg(Color::DarkGray),
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

fn draw_help_tab(f: &mut Frame, area: Rect) {
    let help = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Keyboard Shortcuts",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from("  1        Switch to Repos tab"),
        Line::from("  2        Switch to Security tab"),
        Line::from("  ?        Show this help"),
        Line::from("  Tab      Next system"),
        Line::from("  Sh+Tab   Previous system"),
        Line::from("  Enter/l  Load repos for current system"),
        Line::from("  j / ↓    Move down"),
        Line::from("  k / ↑    Move up"),
        Line::from("  /        Filter repos by name"),
        Line::from("  Esc      Clear filter"),
        Line::from("  q        Quit"),
        Line::from(""),
        Line::from(Span::styled(
            "  About",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from("  Ward - GitHub repository management for developers."),
        Line::from("  plan > apply > verify."),
    ];

    let widget = Paragraph::new(help).block(Block::default().borders(Borders::ALL).title(" Help "));

    f.render_widget(widget, area);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let sys_name = app
        .systems
        .get(app.selected_system)
        .map(|(id, name)| format!("{name} ({id})"))
        .unwrap_or_else(|| "none".to_owned());

    let status = Line::from(vec![
        Span::styled(" System: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&sys_name, Style::default().fg(Color::Cyan)),
        Span::raw("  │  "),
        Span::styled(&app.status_msg, Style::default().fg(Color::DarkGray)),
        Span::raw("  │  "),
        Span::styled(
            "q:quit  Tab:system  Enter:load  /:filter",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let widget = Paragraph::new(status).block(Block::default().borders(Borders::TOP));

    f.render_widget(widget, area);
}
