use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;

use crate::config::Manifest;
use crate::github::Client;
use crate::github::dependency_graph::{DependencyGraphAudit, DependencyGraphStatus};
use crate::github::repos::Repository;

#[derive(Args)]
pub struct AuditCommand {
    /// Output format (table or json)
    #[arg(long, default_value = "table")]
    format: String,
}

#[derive(Debug, Serialize)]
struct AuditReport {
    generated_at: String,
    organization: String,
    repositories: Vec<RepoAudit>,
}

#[derive(Debug, Serialize)]
struct RepoAudit {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_id: Option<String>,
    description: Option<String>,
    default_branch: String,
    security: SecurityAudit,
    dependency_graph: DependencyGraphAudit,
    settings: SettingsAudit,
}

#[derive(Debug, Default, Serialize)]
struct SecurityAudit {
    dependabot_alerts: bool,
    dependabot_security_updates: bool,
    secret_scanning: bool,
    secret_scanning_ai: bool,
    push_protection: bool,
    has_dependabot_config: bool,
    has_codeql: bool,
    alert_counts: AlertCounts,
}

#[derive(Debug, Default, Serialize)]
struct AlertCounts {
    critical: u32,
    high: u32,
    medium: u32,
    low: u32,
}

#[derive(Debug, Default, Serialize)]
struct SettingsAudit {
    has_copilot_review_ruleset: bool,
    has_copilot_instructions: bool,
}

impl AuditCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        let (repos, system_id, scope_label) = resolve_repos(client, manifest, system, repo).await?;

        println!();
        println!(
            "  {} Full audit: {} repo(s) in {}",
            style("[..]").bold(),
            repos.len(),
            style(scope_label).cyan()
        );

        let mut audits = Vec::new();

        for repo in &repos {
            tracing::info!("Auditing {}...", repo.name);
            let audit = audit_repo(client, repo, system_id.as_deref()).await?;
            audits.push(audit);
        }

        if self.format == "json" {
            let report = AuditReport {
                generated_at: chrono::Utc::now().to_rfc3339(),
                organization: client.org().to_owned(),
                repositories: audits,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print_table(&audits);
        }

        Ok(())
    }
}

async fn resolve_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<(Vec<Repository>, Option<String>, String)> {
    if let Some(repo_name) = repo {
        let repo = client.get_repo(repo_name).await?;
        return Ok((vec![repo], None, format!("repository {repo_name}")));
    }

    let sys =
        system.ok_or_else(|| anyhow::anyhow!("Either --system or --repo is required for audit"))?;
    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(
            sys,
            manifest.matches_prefix_for_system(sys),
            &excludes,
            &explicit,
        )
        .await?;
    Ok((repos, Some(sys.to_owned()), format!("system {sys}")))
}

async fn audit_repo(
    client: &Client,
    repo_info: &Repository,
    system_id: Option<&str>,
) -> Result<RepoAudit> {
    let repo = repo_info.name.as_str();
    let security_state = client
        .get_security_state_with_repo_data(repo, repo_info.security_and_analysis.as_ref())
        .await?;

    let has_dependabot_config = client
        .get_file(repo, ".github/dependabot.yml", None)
        .await?
        .is_some();
    let has_codeql = client
        .get_file(repo, ".github/workflows/codeql.yml", None)
        .await?
        .is_some();

    let rulesets = client.list_rulesets(repo).await.unwrap_or_default();
    let has_copilot_review = rulesets.iter().any(|r| r.name == "Copilot Code Review");
    let has_copilot_instructions = client
        .get_file(repo, ".github/copilot-instructions.md", None)
        .await?
        .is_some();

    let alert_counts = get_alert_counts(client, repo).await.unwrap_or_default();
    let dependency_graph = client.audit_dependency_graph(repo).await;

    Ok(RepoAudit {
        name: repo.to_owned(),
        system_id: system_id.map(str::to_owned),
        description: repo_info.description.clone(),
        default_branch: repo_info.default_branch.clone(),
        security: SecurityAudit {
            dependabot_alerts: security_state.dependabot_alerts,
            dependabot_security_updates: security_state.dependabot_security_updates,
            secret_scanning: security_state.secret_scanning,
            secret_scanning_ai: security_state.secret_scanning_ai_detection,
            push_protection: security_state.push_protection,
            has_dependabot_config,
            has_codeql,
            alert_counts,
        },
        dependency_graph,
        settings: SettingsAudit {
            has_copilot_review_ruleset: has_copilot_review,
            has_copilot_instructions,
        },
    })
}

async fn get_alert_counts(client: &Client, repo: &str) -> Result<AlertCounts> {
    let mut counts = AlertCounts::default();

    for severity in ["critical", "high", "medium", "low"] {
        let resp = client
            .get(&format!(
                "/repos/{}/{repo}/dependabot/alerts?state=open&severity={severity}&per_page=1",
                client.org()
            ))
            .await?;

        if resp.status().is_success() {
            let alerts: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
            let count = if alerts.is_empty() { 0 } else { 1 };

            match severity {
                "critical" => counts.critical = count,
                "high" => counts.high = count,
                "medium" => counts.medium = count,
                "low" => counts.low = count,
                _ => {}
            }
        }
    }

    let resp = client
        .get(&format!(
            "/repos/{}/{repo}/dependabot/alerts?state=open&per_page=100",
            client.org()
        ))
        .await?;

    if resp.status().is_success() {
        let alerts: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
        counts = AlertCounts::default();
        for alert in &alerts {
            let severity = alert
                .get("security_vulnerability")
                .and_then(|v| v.get("severity"))
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            match severity {
                "critical" => counts.critical += 1,
                "high" => counts.high += 1,
                "medium" => counts.medium += 1,
                "low" => counts.low += 1,
                _ => {}
            }
        }
    }

    Ok(counts)
}

fn print_table(audits: &[RepoAudit]) {
    use tabled::builder::Builder;
    use tabled::settings::object::{Columns, Rows};
    use tabled::settings::{Alignment, Modify, Style};

    let mut builder = Builder::default();
    builder.push_record([
        "Repository",
        "Dep.A",
        "SecSc",
        "Push",
        "DBot",
        "CQL",
        "SBOM",
        "CopRv",
        "Alert",
    ]);

    let mut total_alerts = 0u32;
    let mut fully_secured = 0;
    let mut dependency_graph_available = 0;

    let icon = |b: bool| {
        if b {
            format!("{}", style("[ok]").green())
        } else {
            format!("{}", style("[!!]").red())
        }
    };

    for a in audits {
        let alert_total = a.security.alert_counts.critical
            + a.security.alert_counts.high
            + a.security.alert_counts.medium
            + a.security.alert_counts.low;
        total_alerts += alert_total;

        let alert_str = if alert_total == 0 {
            format!("{}", style("0").green())
        } else if a.security.alert_counts.critical > 0 {
            format!("{}", style(alert_total).red().bold())
        } else if a.security.alert_counts.high > 0 {
            format!("{}", style(alert_total).yellow().bold())
        } else {
            format!("{}", style(alert_total).yellow())
        };

        let all_security = a.security.dependabot_alerts
            && a.security.secret_scanning
            && a.security.push_protection
            && a.security.has_dependabot_config
            && a.security.has_codeql;

        if all_security {
            fully_secured += 1;
        }

        let dependency_graph_icon = match a.dependency_graph.status {
            DependencyGraphStatus::Available => {
                dependency_graph_available += 1;
                format!("{}", style("[ok]").green())
            }
            DependencyGraphStatus::Empty => format!("{}", style("[--]").yellow()),
            DependencyGraphStatus::Unavailable => format!("{}", style("[!!]").red()),
            DependencyGraphStatus::Unknown => format!("{}", style("[??]").yellow()),
        };

        builder.push_record([
            a.name.clone(),
            icon(a.security.dependabot_alerts),
            icon(a.security.secret_scanning),
            icon(a.security.push_protection),
            icon(a.security.has_dependabot_config),
            icon(a.security.has_codeql),
            dependency_graph_icon,
            icon(a.settings.has_copilot_review_ruleset),
            alert_str,
        ]);
    }

    let table = builder
        .build()
        .with(Style::blank())
        .with(
            Modify::new(Rows::first()).with(tabled::settings::Format::content(|s| {
                format!("{}", style(s).bold().underlined())
            })),
        )
        .with(Modify::new(Columns::new(..)).with(Alignment::left()))
        .to_string();

    println!();
    for line in table.lines() {
        println!("  {line}");
    }

    println!();
    println!(
        "  {} repos audited | {} fully secured | {} SBOM available | {} total open alerts",
        style(audits.len()).bold(),
        style(fully_secured).green().bold(),
        style(dependency_graph_available).green().bold(),
        if total_alerts > 0 {
            style(total_alerts).red().bold()
        } else {
            style(total_alerts).green().bold()
        }
    );
}

#[cfg(test)]
mod tests {
    use tabled::builder::Builder;
    use tabled::settings::object::Columns;
    use tabled::settings::{Alignment, Modify, Style};

    fn strip_ansi(s: &str) -> String {
        let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
        re.replace_all(s, "").to_string()
    }

    #[test]
    fn test_table_columns_align_with_ansi_codes() {
        let ok = format!("{}", console::style("[ok]").green());
        let fail = format!("{}", console::style("[!!]").red());

        let mut builder = Builder::default();
        builder.push_record(["Name", "Status", "Value"]);
        builder.push_record(["short", &ok, "100"]);
        builder.push_record(["a-very-long-repository-name", &fail, "0"]);
        builder.push_record(["medium-name", &ok, "42"]);

        let table = builder
            .build()
            .with(Style::blank())
            .with(Modify::new(Columns::new(..)).with(Alignment::left()))
            .to_string();

        let lines: Vec<&str> = table.lines().collect();
        assert!(lines.len() >= 4, "should have header + 3 data rows");

        // Verify all lines produce consistent visible widths per column.
        // The ansi feature ensures that ANSI escapes don't inflate column width.
        // Strip ANSI and check that the plain-text column positions are consistent.
        let stripped: Vec<String> = lines.iter().copied().map(strip_ansi).collect();
        let header_len = stripped[0].len();

        for (i, line) in stripped.iter().enumerate().skip(1) {
            assert_eq!(
                line.len(),
                header_len,
                "row {i} visible width ({}) != header width ({header_len}): '{line}'",
                line.len()
            );
        }
    }

    #[test]
    fn test_table_handles_empty_data() {
        let mut builder = Builder::default();
        builder.push_record(["Name", "Status"]);
        // No data rows

        let table = builder.build().with(Style::blank()).to_string();
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 1, "header-only table should have 1 line");
    }

    #[test]
    fn test_table_handles_long_repo_names() {
        let long_name = "s07439-party-customer-service-operations-extremely-long-name";
        let ok = format!("{}", console::style("[ok]").green());

        let mut builder = Builder::default();
        builder.push_record(["Repository", "Status"]);
        builder.push_record([long_name, &ok]);
        builder.push_record(["short", &ok]);

        let table = builder
            .build()
            .with(Style::blank())
            .with(Modify::new(Columns::new(..)).with(Alignment::left()))
            .to_string();

        let stripped: Vec<String> = table.lines().map(strip_ansi).collect();
        // All rows should still have the same visible width (padded to longest)
        let widths: Vec<usize> = stripped.iter().map(|l| l.len()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "all rows should have same visible width, got: {widths:?}"
        );
    }
}
