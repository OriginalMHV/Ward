use anyhow::{Context, Result};
use clap::Args;
use console::style;
use reqwest::StatusCode;
use serde::Serialize;

use crate::config::Manifest;
use crate::github::Client;
use crate::github::dependency_graph::{DependencyGraphAudit, DependencyGraphStatus};
use crate::github::repos::Repository;
use crate::reconcile::unified;

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
    /// Data that could not be read for this repository, for example a 403.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unavailable: Vec<String>,
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

        let json_output = is_json_format(&self.format);
        if !json_output {
            println!();
            println!(
                "  {} Full audit: {} repo(s) in {}",
                style("[..]").bold(),
                repos.len(),
                style(scope_label).cyan()
            );
        }

        let mut audits = Vec::new();

        for repo in &repos {
            tracing::info!("Auditing {}...", repo.name);
            let audit = audit_repo(client, repo, system_id.as_deref()).await?;
            audits.push(audit);
        }

        if json_output {
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
    if system.is_none() && repo.is_none() {
        anyhow::bail!("Either --system or --repo is required for audit");
    }
    let repos = unified::resolve_target_repos(client, manifest, system, repo).await?;
    let (system_id, label) = match (system, repo) {
        (_, Some(repo_name)) => (None, format!("repository {repo_name}")),
        (sys, None) => (
            sys.map(str::to_owned),
            format!("system {}", sys.unwrap_or_default()),
        ),
    };
    Ok((repos, system_id, label))
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
        .is_some()
        || client
            .get_file(repo, ".github/dependabot.yaml", None)
            .await?
            .is_some();
    let has_codeql = client
        .get_file(repo, ".github/workflows/codeql.yml", None)
        .await?
        .is_some()
        || has_codeql_default_setup(client, repo).await;

    let mut unavailable = Vec::new();
    if !security_state.unknown.is_empty() {
        unavailable.push(format!(
            "security state: {}",
            security_state.unknown.join(", ")
        ));
    }
    let rulesets = match client.list_rulesets(repo).await {
        Ok(rulesets) => rulesets,
        Err(error) => {
            tracing::warn!("Rulesets unavailable for {repo}: {error:#}");
            unavailable.push(format!("rulesets: {error:#}"));
            Vec::new()
        }
    };
    let has_copilot_review = rulesets.iter().any(|r| r.name == "Copilot Code Review");
    let has_copilot_instructions = client
        .get_file(repo, ".github/copilot-instructions.md", None)
        .await?
        .is_some();

    let alert_counts = match get_alert_counts(client, repo).await {
        Ok(counts) => counts,
        Err(error) => {
            tracing::warn!("Dependabot alerts unavailable for {repo}: {error:#}");
            unavailable.push(format!("dependabot alerts: {error:#}"));
            AlertCounts::default()
        }
    };
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
        unavailable,
    })
}

/// Default setup has no workflow file. An unreadable endpoint counts as not configured.
async fn has_codeql_default_setup(client: &Client, repo: &str) -> bool {
    let path = format!("/repos/{}/{repo}/code-scanning/default-setup", client.org());
    let Ok(response) = client.get(&path).await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| {
            body.get("state")?
                .as_str()
                .map(|state| state == "configured")
        })
        .unwrap_or(false)
}

async fn get_alert_counts(client: &Client, repo: &str) -> Result<AlertCounts> {
    let mut path = format!(
        "/repos/{}/{repo}/dependabot/alerts?state=open&per_page=100",
        client.org()
    );
    let mut counts = AlertCounts::default();

    loop {
        let response = client
            .get(&path)
            .await
            .with_context(|| format!("GET {path} for repository {repo} failed"))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(AlertCounts::default());
        }
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "GET {path} for repository {repo} returned unexpected HTTP status {status}"
            ));
        }
        let next = next_page_path(response.headers());

        let alerts: Vec<serde_json::Value> = response.json().await.with_context(|| {
            format!("Failed to decode JSON from GET {path} for repository {repo}")
        })?;

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

        match next {
            Some(next) => path = next,
            None => return Ok(counts),
        }
    }
}

/// Dependabot alerts use cursor pagination, so follow the `Link: rel="next"`
/// header instead of page numbers. Returns the path and query of the next page.
fn next_page_path(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let link = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        if !rel.contains("rel=\"next\"") {
            return None;
        }
        let url = url.trim().trim_start_matches('<').trim_end_matches('>');
        let after_scheme = url.split_once("://")?.1;
        let path_start = after_scheme.find('/')?;
        Some(after_scheme[path_start..].to_owned())
    })
}

fn is_json_format(format: &str) -> bool {
    format == "json"
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

        let alert_str = if a
            .unavailable
            .iter()
            .any(|entry| entry.starts_with("dependabot alerts"))
        {
            format!("{}", style("?").yellow())
        } else if alert_total == 0 {
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

    for audit in audits.iter().filter(|a| !a.unavailable.is_empty()) {
        println!();
        println!(
            "  {} {}: data unavailable",
            style("[??]").yellow(),
            audit.name
        );
        for entry in &audit.unavailable {
            println!("      {entry}");
        }
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
    use serde_json::json;
    use tabled::builder::Builder;
    use tabled::settings::object::Columns;
    use tabled::settings::{Alignment, Modify, Style};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{get_alert_counts, is_json_format};
    use crate::github::Client;

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

    #[test]
    fn json_format_suppresses_human_progress_output() {
        assert!(is_json_format("json"));
        assert!(!is_json_format("table"));
    }

    #[tokio::test]
    async fn malformed_alert_json_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/dependabot/alerts"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let error = get_alert_counts(&client, "my-repo")
            .await
            .expect_err("malformed alert JSON must fail the audit");

        let message = format!("{error:#}");
        assert!(message.contains("Failed to decode JSON"));
        assert!(message.contains("my-repo"));
    }

    #[tokio::test]
    async fn missing_alert_endpoint_is_treated_as_zero_counts() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/dependabot/alerts"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let counts = get_alert_counts(&client, "my-repo")
            .await
            .expect("missing alert endpoint should be treated as empty");

        assert_eq!(counts.critical, 0);
        assert_eq!(counts.high, 0);
        assert_eq!(counts.medium, 0);
        assert_eq!(counts.low, 0);
    }

    #[tokio::test]
    async fn failed_alert_request_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/dependabot/alerts"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "message": "server error"
            })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let error = get_alert_counts(&client, "my-repo")
            .await
            .expect_err("failed alert requests must fail the audit");

        let message = format!("{error:#}");
        assert!(message.contains("GET"));
        assert!(message.contains("my-repo"));
        assert!(message.contains("/dependabot/alerts"));
    }

    #[tokio::test]
    async fn alert_counts_follow_link_header_pages() {
        let server = MockServer::start().await;
        let alert = |severity: &str| json!({"security_vulnerability": {"severity": severity}});
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/dependabot/alerts"))
            .and(wiremock::matchers::query_param("after", "cursor1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([alert("high")])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/dependabot/alerts"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "link",
                        format!(
                            "<{}/repos/test-org/my-repo/dependabot/alerts?state=open&per_page=100&after=cursor1>; rel=\"next\"",
                            server.uri()
                        ),
                    )
                    .set_body_json(json!([alert("critical"), alert("low")])),
            )
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let counts = get_alert_counts(&client, "my-repo").await.unwrap();

        assert_eq!((counts.critical, counts.high, counts.low), (1, 1, 1));
    }

    #[tokio::test]
    async fn forbidden_alerts_and_rulesets_do_not_abort_the_audit() {
        let server = MockServer::start().await;
        for suffix in ["dependabot/alerts", "rulesets"] {
            Mock::given(method("GET"))
                .and(path(format!("/repos/test-org/my-repo/{suffix}")))
                .respond_with(
                    ResponseTemplate::new(403).set_body_json(json!({"message": "Forbidden"})),
                )
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path(
                "/repos/test-org/my-repo/contents/.github/dependabot.yaml",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "dependabot.yaml",
                "path": ".github/dependabot.yaml",
                "sha": "abc",
                "type": "file",
                "content": "dmVyc2lvbjogMg==",
                "encoding": "base64"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let repo: crate::github::repos::Repository = serde_json::from_value(json!({
            "name": "my-repo",
            "full_name": "test-org/my-repo",
            "archived": false,
            "default_branch": "main",
            "visibility": "private"
        }))
        .unwrap();

        let audit = super::audit_repo(&client, &repo, None).await.unwrap();
        let joined = audit.unavailable.join("\n");
        assert!(joined.contains("rulesets:"), "{joined}");
        assert!(joined.contains("dependabot alerts:"), "{joined}");
        assert!(audit.security.has_dependabot_config);
    }

    #[tokio::test]
    async fn codeql_default_setup_counts_as_codeql() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/code-scanning/default-setup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"state": "configured"})))
            .mount(&server)
            .await;
        let client = Client::new_for_test("test-org", &server.uri());
        assert!(super::has_codeql_default_setup(&client, "my-repo").await);

        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&other)
            .await;
        let client = Client::new_for_test("test-org", &other.uri());
        assert!(!super::has_codeql_default_setup(&client, "my-repo").await);
    }
}
