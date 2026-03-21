use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::CrosstermBackend,
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};

use crate::config::Manifest;
use crate::github::repos::Repository;
use crate::github::security::SecurityState;
use crate::github::Client;

enum Tab {
    Repos,
    Security,
    Help,
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
}

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
            status_msg: "Press 'l' to load repos for selected system".to_owned(),
            systems,
            selected_system: 0,
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
        anyhow::bail!("No systems defined in ward.toml — add [[systems]] entries first");
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

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &Client,
    manifest: &Manifest,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
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
                        load_repos(app, client, manifest).await?;
                    }
                    KeyCode::Char('s') => {
                        // Cycle through systems
                        if !app.systems.is_empty() {
                            app.selected_system =
                                (app.selected_system + 1) % app.systems.len();
                            app.status_msg = format!(
                                "System: {} — press 'l' to load",
                                app.systems[app.selected_system].1
                            );
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
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

async fn load_repos(app: &mut App, client: &Client, manifest: &Manifest) -> Result<()> {
    if app.systems.is_empty() {
        return Ok(());
    }

    let (sys_id, sys_name) = &app.systems[app.selected_system];
    app.status_msg = format!("Loading repos for {sys_name}...");

    let excludes = manifest.exclude_patterns_for_system(sys_id);
    let repos = client.list_repos_for_system(sys_id, &excludes).await?;

    let mut entries = Vec::new();
    for repo in repos {
        let security = client.get_security_state(&repo.name).await.ok();
        entries.push(RepoEntry { repo, security });
    }

    app.status_msg = format!(
        "Loaded {} repos for {} ({})",
        entries.len(),
        sys_name,
        sys_id
    );
    app.repos = entries;
    app.list_state.select(Some(0));

    Ok(())
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header + tabs
            Constraint::Min(10),  // main content
            Constraint::Length(3), // status bar
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
                .title(" Ward — GitHub Repository Management "),
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

    // Repo list
    let filtered = app.filtered_repos();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|entry| {
            let lang = entry
                .repo
                .language
                .as_deref()
                .unwrap_or("-");
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
                Span::styled(
                    entry.repo.name.clone(),
                    Style::default().fg(Color::White),
                ),
                Span::raw("  "),
                Span::styled(lang, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let title = if app.is_filtering {
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

    // Detail panel
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
                Span::raw("  Language:  "),
                Span::styled(
                    entry.repo.language.as_deref().unwrap_or("-"),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::raw("  Branch:    "),
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
            lines.push(Line::from("  (not loaded)"));
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
        let msg = Paragraph::new("  Press 'l' to load repos, then switch to this tab.")
            .block(Block::default().borders(Borders::ALL).title(" Security Overview "));
        f.render_widget(msg, area);
        return;
    }

    let mut lines = vec![
        Line::from(Span::styled(
            format!("  {:35} {:6} {:6} {:6} {:6} {:6}", "Repository", "Dep.A", "SecSc", "AI", "Push", "SecUp"),
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
    ];

    let mut secured = 0;
    let mut issues = 0;

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
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Summary: {secured} secured, {issues} need attention"),
        Style::default()
            .fg(if issues > 0 { Color::Yellow } else { Color::Green })
            .bold(),
    )));

    let widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Security Overview "))
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

fn draw_help_tab(f: &mut Frame, area: Rect) {
    let help = vec![
        Line::from(""),
        Line::from(Span::styled("  Keyboard Shortcuts", Style::default().fg(Color::Cyan).bold())),
        Line::from(""),
        Line::from("  1       Switch to Repos tab"),
        Line::from("  2       Switch to Security tab"),
        Line::from("  ?       Show this help"),
        Line::from("  s       Cycle through systems"),
        Line::from("  l       Load repos for current system"),
        Line::from("  Enter   Load repos for current system"),
        Line::from("  j / ↓   Move down"),
        Line::from("  k / ↑   Move up"),
        Line::from("  /       Filter repos by name"),
        Line::from("  Esc     Clear filter"),
        Line::from("  q       Quit"),
        Line::from(""),
        Line::from(Span::styled("  About", Style::default().fg(Color::Cyan).bold())),
        Line::from(""),
        Line::from("  Ward — GitHub repository management for developers."),
        Line::from("  plan → apply → verify."),
    ];

    let widget = Paragraph::new(help)
        .block(Block::default().borders(Borders::ALL).title(" Help "));

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
        Span::styled("q:quit  s:system  l:load  /:filter", Style::default().fg(Color::DarkGray)),
    ]);

    let widget = Paragraph::new(status)
        .block(Block::default().borders(Borders::TOP));

    f.render_widget(widget, area);
}
