use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Tabs, Wrap},
};

use super::state::{App, Tab, has_loaded_repo_data, repo_is_healthy};
use crate::github::dependency_graph::DependencyGraphStatus;

pub(super) fn dependency_graph_indicator(
    status: &DependencyGraphStatus,
) -> (&'static str, Color, &'static str) {
    match status {
        DependencyGraphStatus::Available => ("[Y]", Color::Green, "SBOM available"),
        DependencyGraphStatus::Empty => ("[-]", Color::Yellow, "SBOM empty"),
        DependencyGraphStatus::Unavailable => ("[N]", Color::Red, "SBOM unavailable"),
        DependencyGraphStatus::Unknown => ("[?]", Color::Yellow, "SBOM unknown"),
    }
}

pub(super) fn draw(f: &mut Frame, app: &App) {
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
            let (indicator, color) =
                match (entry.security.as_ref(), entry.dependency_graph.as_ref()) {
                    (Some(s), Some(dep)) if repo_is_healthy(s, dep) => ("[ok]", Color::Green),
                    (Some(_), Some(_)) => ("[!!]", Color::Yellow),
                    _ => ("[..]", Color::DarkGray),
                };

            ListItem::new(Line::from(vec![
                Span::styled(indicator, Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(entry.repo.name.clone(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(lang, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let sys_label = app
        .systems
        .get(app.selected_system)
        .map(|(_, name)| name.as_str())
        .unwrap_or("?");
    let title = if app.loading {
        format!(" {sys_label} [{}] loading... ", app.spinner_char())
    } else if app.is_filtering {
        format!(" {sys_label} (filter: {}_) ", app.filter)
    } else if app.filter.is_empty() {
        format!(" {sys_label} ({}) ", filtered.len())
    } else {
        format!(" {sys_label} ({}/{}) ", filtered.len(), app.repos.len())
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
        let dependency_graph = entry.dependency_graph.as_ref();
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
        ];

        if let Some(ref desc) = entry.repo.description
            && !desc.is_empty()
        {
            let max_chars = 80;
            if desc.len() > max_chars {
                let first = &desc[..max_chars];
                let rest = &desc[max_chars..];
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(first, Style::default().fg(Color::DarkGray)),
                ]));
                let trimmed = if rest.len() > max_chars {
                    format!("{}...", &rest[..max_chars.min(rest.len())])
                } else {
                    rest.to_owned()
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(trimmed, Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(desc.clone(), Style::default().fg(Color::DarkGray)),
                ]));
            }
            lines.push(Line::from(""));
        }

        if entry.repo.archived {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("[ARCHIVED]", Style::default().fg(Color::Red).bold()),
            ]));
            lines.push(Line::from(""));
        }

        lines.extend([
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
        ]);

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

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Dependency Graph",
            Style::default().fg(Color::Cyan).bold(),
        )));

        if let Some(dep) = dependency_graph {
            let (status_text, status_color, status_label) = dependency_graph_indicator(&dep.status);
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(status_text, Style::default().fg(status_color)),
                Span::raw(format!(" {status_label}")),
            ]));
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(dep.reason.clone(), Style::default().fg(Color::DarkGray)),
            ]));
            if let Some(count) = dep.dependency_count {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(
                        format!("Dependencies: {count}"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            if let Some(ref generated_at) = dep.sbom_generated_at {
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(
                        format!("Generated: {generated_at}"),
                        Style::default().fg(Color::DarkGray),
                    ),
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
    let sys_label = app
        .systems
        .get(app.selected_system)
        .map(|(_, name)| name.as_str())
        .unwrap_or("?");
    let sys_id = app
        .systems
        .get(app.selected_system)
        .map(|(id, _)| id.as_str())
        .unwrap_or("");

    if app.repos.is_empty() {
        let msg = Paragraph::new("  Press Enter to load repos, then switch to this tab.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Security: {sys_label} ")),
        );
        f.render_widget(msg, area);
        return;
    }

    let prefix = format!("{sys_id}-");
    let all_share_prefix = app.repos.iter().all(|e| e.repo.name.starts_with(&prefix));

    let display_name = |name: &str| -> String {
        if all_share_prefix {
            name.strip_prefix(&prefix).unwrap_or(name).to_owned()
        } else {
            name.to_owned()
        }
    };

    let max_name_len = app
        .repos
        .iter()
        .map(|e| display_name(&e.repo.name).len())
        .max()
        .unwrap_or(20);
    let col_repo: usize = max_name_len.clamp(10, 50) + 2;

    let truncate_name = |name: &str| -> String {
        let dname = display_name(name);
        if dname.len() > col_repo {
            format!("{}...", &dname[..col_repo - 3])
        } else {
            dname
        }
    };

    let builtin_headers = [
        "Repository",
        "Dependabot",
        "Secret Scanning",
        "AI Detection",
        "Push Protection",
        "Security Updates",
        "SBOM",
    ];
    let header_style = Style::default().fg(Color::Cyan).bold();

    let mut header_cells: Vec<Cell> = builtin_headers
        .iter()
        .map(|h| Cell::from(*h).style(header_style))
        .collect();
    for check in &app.custom_checks {
        header_cells.push(Cell::from(check.name()).style(header_style));
    }

    let header = Row::new(header_cells)
        .style(Style::default())
        .bottom_margin(0);

    let mut healthy = 0;
    let mut issues = 0;
    let mut pending = 0;

    let rows: Vec<Row> = app
        .repos
        .iter()
        .enumerate()
        .map(|(row_idx, entry)| {
            let row_bg = if row_idx % 2 == 1 {
                Color::Rgb(30, 30, 40)
            } else {
                Color::Reset
            };
            let row_style = Style::default().bg(row_bg);

            let icon = |b: bool| -> Cell {
                if b {
                    Cell::from(" [Y]").style(Style::default().fg(Color::Green).bg(row_bg))
                } else {
                    Cell::from(" [N]").style(Style::default().fg(Color::Red).bg(row_bg))
                }
            };

            let loading_cell =
                Cell::from(" [..]").style(Style::default().fg(Color::DarkGray).bg(row_bg));

            if let (Some(s), Some(dep)) = (entry.security.as_ref(), entry.dependency_graph.as_ref())
            {
                if repo_is_healthy(s, dep) {
                    healthy += 1;
                } else {
                    issues += 1;
                }

                let (dep_text, dep_color, _) = dependency_graph_indicator(&dep.status);
                let dependency_cell = Cell::from(format!(" {dep_text}"))
                    .style(Style::default().fg(dep_color).bg(row_bg));

                let mut cells = vec![
                    Cell::from(truncate_name(&entry.repo.name)).style(Style::default().bg(row_bg)),
                    icon(s.dependabot_alerts),
                    icon(s.secret_scanning),
                    icon(s.secret_scanning_ai_detection),
                    icon(s.push_protection),
                    icon(s.dependabot_security_updates),
                    dependency_cell,
                ];

                for result in &entry.custom_checks {
                    cells.push(match result {
                        Some(val) => icon(*val),
                        None => loading_cell.clone(),
                    });
                }

                Row::new(cells).style(row_style)
            } else {
                pending += 1;
                let builtin_count = 7;
                let total_cols = builtin_count + app.custom_checks.len();
                let mut cells = Vec::with_capacity(total_cols);
                cells.push(
                    Cell::from(truncate_name(&entry.repo.name))
                        .style(Style::default().fg(Color::DarkGray).bg(row_bg)),
                );
                for _ in 1..total_cols {
                    cells.push(loading_cell.clone());
                }
                Row::new(cells).style(row_style)
            }
        })
        .collect();

    let mut widths = vec![
        Constraint::Min(col_repo as u16),
        Constraint::Min(12),
        Constraint::Min(17),
        Constraint::Min(14),
        Constraint::Min(17),
        Constraint::Min(18),
        Constraint::Min(10),
    ];
    for check in &app.custom_checks {
        let w = check.name().len().max(10) + 2;
        widths.push(Constraint::Min(w as u16));
    }

    let prefix_note = if all_share_prefix {
        format!(" (prefix {prefix} stripped)")
    } else {
        String::new()
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Security: {sys_label}{prefix_note} ")),
        )
        .column_spacing(1);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(area);

    f.render_widget(table, layout[0]);

    let summary = if pending > 0 {
        format!("  {healthy} healthy, {issues} need attention, {pending} loading...")
    } else {
        format!("  {healthy} healthy, {issues} need attention")
    };
    let summary_widget = Paragraph::new(Line::from(Span::styled(
        summary,
        Style::default()
            .fg(if issues > 0 {
                Color::Yellow
            } else {
                Color::Green
            })
            .bold(),
    )));
    f.render_widget(summary_widget, layout[1]);
}

fn draw_actions_tab(f: &mut Frame, area: Rect) {
    let key_style = Style::default().fg(Color::Yellow).bold();
    let heading = Style::default().fg(Color::Cyan).bold();
    let dim = Style::default().fg(Color::DarkGray);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Available Actions (on selected repo)",
            heading,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  a", key_style),
            Span::raw("    Apply security settings"),
        ]),
        Line::from(vec![
            Span::styled("  p", key_style),
            Span::raw("    Apply branch protection"),
        ]),
        Line::from(vec![
            Span::styled("  t", key_style),
            Span::raw("    Deploy template (sub-menu: dependabot/codeql/submission)"),
        ]),
        Line::from(vec![
            Span::styled("  S", key_style),
            Span::raw("    Apply settings (copilot ruleset + instructions)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Bulk Actions (on all filtered repos)",
            heading,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  A", key_style),
            Span::raw("    Apply security to all filtered repos"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Navigation", heading)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  r", key_style),
            Span::raw("    Refresh/reload current system"),
        ]),
        Line::from(vec![
            Span::styled("  R", key_style),
            Span::raw("    Force reload (ignore cache)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Templates deploy a branch + commit + PR for the selected repo.",
            dim,
        )),
        Line::from(Span::styled(
            "  Settings creates a copilot review ruleset and instructions file.",
            dim,
        )),
    ];

    let widget =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Actions "));

    f.render_widget(widget, area);
}

fn draw_help_tab(f: &mut Frame, area: Rect) {
    let heading = Style::default().fg(Color::Cyan).bold();
    let dim = Style::default().fg(Color::DarkGray);
    let key = Style::default().fg(Color::Yellow);

    let block = Block::default().borders(Borders::ALL).title(" Help ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    fn section<'a>(
        title: &'a str,
        entries: &[(&'a str, &'a str)],
        heading: Style,
        dim: Style,
        key_style: Style,
    ) -> Vec<Line<'a>> {
        let mut lines = vec![
            Line::from(Span::styled(format!(" {title}"), heading)),
            Line::from(Span::styled(
                format!(" {}", "\u{2500}".repeat(title.len())),
                dim,
            )),
        ];
        for &(k, desc) in entries {
            lines.push(Line::from(vec![
                Span::styled(format!(" {k:<15}"), key_style),
                Span::raw(desc),
            ]));
        }
        lines.push(Line::from(""));
        lines
    }

    let mut left: Vec<Line> = Vec::new();
    left.push(Line::from(""));
    left.extend(section(
        "Navigation",
        &[
            ("j/k, arrows", "Move up/down"),
            ("Tab/s", "Next system"),
            ("Shift+Tab", "Previous system"),
            ("Enter/l", "Load repos"),
            ("/", "Filter"),
            ("Esc", "Clear filter"),
        ],
        heading,
        dim,
        key,
    ));
    left.extend(section(
        "General",
        &[
            ("q", "Quit"),
            ("r", "Reload"),
            ("R", "Force reload"),
            ("?", "Help"),
        ],
        heading,
        dim,
        key,
    ));
    left.extend(section(
        "Filter Syntax",
        &[
            ("foo", "Show matching"),
            ("!ops", "Hide matching"),
            ("!ops !sys", "Hide both"),
            ("foo !ops", "Show foo, hide ops"),
        ],
        heading,
        dim,
        key,
    ));
    left.push(Line::from(Span::styled(
        " Stack ! terms like kanban labels to narrow view.",
        dim,
    )));

    let mut right: Vec<Line> = Vec::new();
    right.push(Line::from(""));
    right.extend(section(
        "Repo Actions",
        &[
            ("a", "Apply security"),
            ("p", "Apply branch protection"),
            ("t", "Deploy template (d/c/s)"),
            ("S", "Apply settings (copilot)"),
        ],
        heading,
        dim,
        key,
    ));
    right.extend(section(
        "Bulk Actions",
        &[("A", "Apply security to all")],
        heading,
        dim,
        key,
    ));
    right.extend(section(
        "Tabs",
        &[
            ("1", "Repos"),
            ("2", "Security"),
            ("3", "Actions"),
            ("?", "Help"),
        ],
        heading,
        dim,
        key,
    ));

    let left_widget = Paragraph::new(left);
    let right_widget = Paragraph::new(right);

    f.render_widget(left_widget, columns[0]);
    f.render_widget(right_widget, columns[1]);
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
        let loaded = app.repos.iter().filter(|r| has_loaded_repo_data(r)).count();
        let total = app.repos.len();
        format!(
            "[{}] Security + SBOM: {loaded}/{total}...",
            app.spinner_char()
        )
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
            "q:quit Tab:sys Enter:load /:filter a/p/t:actions A:bulk",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let status = Line::from(spans);
    let widget = Paragraph::new(status).block(Block::default().borders(Borders::TOP));

    f.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_graph_indicator_matches_status() {
        assert_eq!(
            dependency_graph_indicator(&DependencyGraphStatus::Available),
            ("[Y]", Color::Green, "SBOM available")
        );
        assert_eq!(
            dependency_graph_indicator(&DependencyGraphStatus::Empty),
            ("[-]", Color::Yellow, "SBOM empty")
        );
        assert_eq!(
            dependency_graph_indicator(&DependencyGraphStatus::Unavailable),
            ("[N]", Color::Red, "SBOM unavailable")
        );
        assert_eq!(
            dependency_graph_indicator(&DependencyGraphStatus::Unknown),
            ("[?]", Color::Yellow, "SBOM unknown")
        );
    }
}
