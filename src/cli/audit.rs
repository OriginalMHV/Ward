use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use console::style;
use reqwest::StatusCode;
use serde::Serialize;

use super::args::{OutputArgs, TargetArgs};
use super::output::{self, ok_icon};
use crate::config::Manifest;
use crate::github::Client;
use crate::github::actions::ReadOutcome;
use crate::github::branch_protection::BranchProtectionState;
use crate::github::dependency_graph::{DependencyGraphAudit, DependencyGraphStatus};
use crate::github::repos::Repository;
use crate::reconcile::unified;

const COPILOT_REVIEW_RULE: &str = "copilot_code_review";
const COPILOT_REVIEW_RULESET_NAME: &str = "Copilot Code Review";

/// A section of the audit report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuditCategory {
    Security,
    #[value(alias = "ruleset")]
    Rulesets,
    #[value(name = "branch-protection", alias = "protection")]
    BranchProtection,
    #[value(alias = "teams")]
    Access,
}

#[derive(Args)]
pub struct AuditCommand {
    /// Report only these sections (default: all). Repeat the flag or separate values with commas.
    #[arg(
        long = "category",
        value_name = "CATEGORY",
        value_enum,
        value_delimiter = ',',
        ignore_case = true
    )]
    category: Vec<AuditCategory>,

    #[command(flatten)]
    pub target: TargetArgs,

    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Clone, Copy)]
struct Sections {
    security: bool,
    rulesets: bool,
    branch_protection: bool,
    access: bool,
}

impl Sections {
    fn select(categories: &[AuditCategory]) -> Self {
        let all = categories.is_empty();
        let has = |category| all || categories.contains(&category);
        Self {
            security: has(AuditCategory::Security),
            rulesets: has(AuditCategory::Rulesets),
            branch_protection: has(AuditCategory::BranchProtection),
            access: has(AuditCategory::Access),
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    security: Option<SecurityAudit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependency_graph: Option<DependencyGraphAudit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<SettingsAudit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rulesets: Option<Vec<RulesetAudit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_protection: Option<BranchProtectionAudit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<AccessAudit>,
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

#[derive(Debug, Serialize)]
struct RulesetAudit {
    name: String,
    enforcement: String,
    target: String,
}

#[derive(Debug, Serialize)]
struct BranchProtectionAudit {
    branch: String,
    protected: bool,
    #[serde(flatten)]
    state: BranchProtectionState,
}

#[derive(Debug, Default, Serialize)]
struct AccessAudit {
    teams: Vec<TeamAudit>,
}

#[derive(Debug, Serialize)]
struct TeamAudit {
    slug: String,
    permission: String,
}

impl AuditCommand {
    /// An audit of one section, for the deprecated per-category commands.
    pub(crate) fn for_section(
        section: AuditCategory,
        target: TargetArgs,
        output: OutputArgs,
    ) -> Self {
        Self {
            category: vec![section],
            target,
            output,
        }
    }

    pub async fn run(&self, client: &Client, manifest: &Manifest) -> Result<()> {
        let system = self.target.system.as_deref();
        let repo = self.target.repo.as_deref();
        let (repos, system_id, scope_label) = resolve_repos(client, manifest, system, repo).await?;
        let sections = Sections::select(&self.category);

        let json_output = self.output.is_json();
        eprintln!(
            "  {} Full audit: {} repo(s) in {}",
            style("[..]").bold(),
            repos.len(),
            style(scope_label).cyan()
        );

        let audits = crate::reconcile::map_buffered(&repos, |repo| async {
            tracing::info!("Auditing {}...", repo.name);
            audit_repo(client, repo, system_id.as_deref(), sections).await
        })
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;

        if json_output {
            let report = AuditReport {
                generated_at: chrono::Utc::now().to_rfc3339(),
                organization: client.org().to_owned(),
                repositories: audits,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print_report(&audits, sections);
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
    let repos = unified::resolve_target_repos(client, manifest, system, repo).await?;
    let (system_id, label) = match (system, repo) {
        (_, Some(repo_name)) => (None, format!("repository {repo_name}")),
        (Some(sys), None) => (Some(sys.to_owned()), format!("system {sys}")),
        (None, None) => (None, "all configured systems".to_owned()),
    };
    Ok((repos, system_id, label))
}

async fn audit_repo(
    client: &Client,
    repo_info: &Repository,
    system_id: Option<&str>,
    sections: Sections,
) -> Result<RepoAudit> {
    let repo = repo_info.name.as_str();
    let mut unavailable = Vec::new();

    let (security, dependency_graph) = if sections.security {
        let (security, graph) = audit_security(client, repo_info, &mut unavailable).await?;
        (Some(security), Some(graph))
    } else {
        (None, None)
    };

    let (rulesets, settings) = if sections.rulesets {
        let (rulesets, settings) = audit_rulesets(client, repo_info, &mut unavailable).await?;
        (Some(rulesets), Some(settings))
    } else {
        (None, None)
    };

    let branch_protection = if sections.branch_protection {
        audit_branch_protection(client, repo_info, &mut unavailable).await
    } else {
        None
    };

    let access = if sections.access {
        audit_access(client, repo, &mut unavailable).await?
    } else {
        None
    };

    Ok(RepoAudit {
        name: repo.to_owned(),
        system_id: system_id.map(str::to_owned),
        description: repo_info.description.clone(),
        default_branch: repo_info.default_branch.clone(),
        security,
        dependency_graph,
        settings,
        rulesets,
        branch_protection,
        access,
        unavailable,
    })
}

async fn audit_security(
    client: &Client,
    repo_info: &Repository,
    unavailable: &mut Vec<String>,
) -> Result<(SecurityAudit, DependencyGraphAudit)> {
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

    if !security_state.unknown.is_empty() {
        unavailable.push(format!(
            "security state: {}",
            security_state.unknown.join(", ")
        ));
    }

    let alert_counts = match get_alert_counts(client, repo).await {
        Ok(counts) => counts,
        Err(error) => {
            tracing::warn!("Dependabot alerts unavailable for {repo}: {error:#}");
            unavailable.push(format!("dependabot alerts: {error:#}"));
            AlertCounts::default()
        }
    };
    let dependency_graph = client.audit_dependency_graph(repo).await;

    Ok((
        SecurityAudit {
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
    ))
}

async fn audit_rulesets(
    client: &Client,
    repo_info: &Repository,
    unavailable: &mut Vec<String>,
) -> Result<(Vec<RulesetAudit>, SettingsAudit)> {
    let repo = repo_info.name.as_str();
    let (rulesets, listed) = match client.list_rulesets(repo).await {
        Ok(rulesets) => (rulesets, true),
        Err(error) => {
            tracing::warn!("Rulesets unavailable for {repo}: {error:#}");
            unavailable.push(format!("rulesets: {error:#}"));
            (Vec::new(), false)
        }
    };

    let by_name = rulesets
        .iter()
        .any(|ruleset| ruleset.name == COPILOT_REVIEW_RULESET_NAME);
    // The rules endpoint answers by rule type in one request. Skip it when no
    // ruleset exists, because then no rule can apply.
    let has_copilot_review = if listed && rulesets.is_empty() {
        false
    } else {
        match client
            .list_branch_rule_types(repo, &repo_info.default_branch)
            .await
        {
            Ok(ReadOutcome::Available(types)) => types.iter().any(|t| t == COPILOT_REVIEW_RULE),
            Ok(_) | Err(_) => by_name,
        }
    };
    let has_copilot_instructions = client
        .get_file(repo, ".github/copilot-instructions.md", None)
        .await?
        .is_some();

    let audits = rulesets
        .into_iter()
        .map(|ruleset| RulesetAudit {
            name: ruleset.name,
            enforcement: ruleset.enforcement,
            target: ruleset.target,
        })
        .collect();
    Ok((
        audits,
        SettingsAudit {
            has_copilot_review_ruleset: has_copilot_review,
            has_copilot_instructions,
        },
    ))
}

async fn audit_branch_protection(
    client: &Client,
    repo_info: &Repository,
    unavailable: &mut Vec<String>,
) -> Option<BranchProtectionAudit> {
    let branch = repo_info.default_branch.as_str();
    match client.get_branch_protection(&repo_info.name, branch).await {
        Ok(state) => Some(BranchProtectionAudit {
            branch: branch.to_owned(),
            protected: state.is_some(),
            state: state.unwrap_or_default(),
        }),
        Err(error) => {
            tracing::warn!(
                "Branch protection unavailable for {}: {error:#}",
                repo_info.name
            );
            unavailable.push(format!("branch protection: {error:#}"));
            None
        }
    }
}

async fn audit_access(
    client: &Client,
    repo: &str,
    unavailable: &mut Vec<String>,
) -> Result<Option<AccessAudit>> {
    Ok(match client.list_repo_teams_checked(repo).await? {
        ReadOutcome::Available(teams) => Some(AccessAudit {
            teams: teams
                .iter()
                .map(|team| TeamAudit {
                    slug: team.slug.clone(),
                    permission: team.effective_permission().to_owned(),
                })
                .collect(),
        }),
        ReadOutcome::NotApplicable(reason)
        | ReadOutcome::PermissionDenied(reason)
        | ReadOutcome::Unavailable(reason) => {
            unavailable.push(format!("teams: {reason}"));
            None
        }
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

fn print_report(audits: &[RepoAudit], sections: Sections) {
    if sections.security {
        print_security(audits);
    }
    if sections.rulesets {
        print_rulesets(audits);
    }
    if sections.branch_protection {
        print_branch_protection(audits);
    }
    if sections.access {
        print_access(audits);
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
}

fn print_heading(title: &str) {
    println!();
    println!("  {}", style(title).bold());
}

fn print_security(audits: &[RepoAudit]) {
    use tabled::builder::Builder;

    let mut builder = Builder::default();
    builder.push_record([
        "Repository",
        "Dep.A",
        "Dep.SU",
        "SecSc",
        "AI",
        "Push",
        "DBot",
        "CQL",
        "SBOM",
        "Alert",
    ]);

    let mut total_alerts = 0u32;
    let mut fully_secured = 0;
    let mut dependency_graph_available = 0;
    let mut audited = 0;

    for a in audits {
        let (Some(security), Some(graph)) = (&a.security, &a.dependency_graph) else {
            continue;
        };
        audited += 1;
        let alert_total = security.alert_counts.critical
            + security.alert_counts.high
            + security.alert_counts.medium
            + security.alert_counts.low;
        total_alerts += alert_total;

        let alert_str = if a
            .unavailable
            .iter()
            .any(|entry| entry.starts_with("dependabot alerts"))
        {
            format!("{}", style("?").yellow())
        } else if alert_total == 0 {
            format!("{}", style("0").green())
        } else if security.alert_counts.critical > 0 {
            format!("{}", style(alert_total).red().bold())
        } else if security.alert_counts.high > 0 {
            format!("{}", style(alert_total).yellow().bold())
        } else {
            format!("{}", style(alert_total).yellow())
        };

        let all_security = security.dependabot_alerts
            && security.secret_scanning
            && security.push_protection
            && security.has_dependabot_config
            && security.has_codeql;

        if all_security {
            fully_secured += 1;
        }

        let dependency_graph_icon = match graph.status {
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
            ok_icon(security.dependabot_alerts),
            ok_icon(security.dependabot_security_updates),
            ok_icon(security.secret_scanning),
            ok_icon(security.secret_scanning_ai),
            ok_icon(security.push_protection),
            ok_icon(security.has_dependabot_config),
            ok_icon(security.has_codeql),
            dependency_graph_icon,
            alert_str,
        ]);
    }

    print_heading("Security");
    println!();
    output::print_table(builder);
    println!();
    println!(
        "  {} repos audited | {} fully secured | {} SBOM available | {} total open alerts",
        style(audited).bold(),
        style(fully_secured).green().bold(),
        style(dependency_graph_available).green().bold(),
        if total_alerts > 0 {
            style(total_alerts).red().bold()
        } else {
            style(total_alerts).green().bold()
        }
    );
}

fn print_rulesets(audits: &[RepoAudit]) {
    use tabled::builder::Builder;

    let mut builder = Builder::default();
    builder.push_record(["Repository", "Ruleset", "Enforcement", "CopRv"]);

    for a in audits {
        let (Some(rulesets), Some(settings)) = (&a.rulesets, &a.settings) else {
            continue;
        };
        let copilot = ok_icon(settings.has_copilot_review_ruleset);
        if rulesets.is_empty() {
            builder.push_record([
                a.name.clone(),
                style("(none)").dim().to_string(),
                String::new(),
                copilot,
            ]);
            continue;
        }
        for (index, ruleset) in rulesets.iter().enumerate() {
            builder.push_record([
                if index == 0 {
                    a.name.clone()
                } else {
                    String::new()
                },
                ruleset.name.clone(),
                ruleset.enforcement.clone(),
                if index == 0 {
                    copilot.clone()
                } else {
                    String::new()
                },
            ]);
        }
    }

    print_heading("Rulesets");
    println!();
    output::print_table(builder);
}

fn print_branch_protection(audits: &[RepoAudit]) {
    use tabled::builder::Builder;

    let mut builder = Builder::default();
    builder.push_record([
        "Repository",
        "Branch",
        "PR Rev",
        "Approvals",
        "Stale",
        "Admins",
        "Linear",
        "Force",
    ]);

    let mut protected = 0;
    let mut unprotected = 0;
    for a in audits {
        let Some(bp) = &a.branch_protection else {
            continue;
        };
        if bp.state.required_pull_request_reviews {
            protected += 1;
        } else {
            unprotected += 1;
        }
        builder.push_record([
            a.name.clone(),
            bp.branch.clone(),
            ok_icon(bp.state.required_pull_request_reviews),
            bp.state.required_approving_review_count.to_string(),
            ok_icon(bp.state.dismiss_stale_reviews),
            ok_icon(bp.state.enforce_admins),
            ok_icon(bp.state.required_linear_history),
            ok_icon(bp.state.allow_force_pushes),
        ]);
    }

    print_heading("Branch protection");
    println!();
    output::print_table(builder);
    println!();
    println!(
        "  {} protected, {} unprotected",
        style(protected).green().bold(),
        if unprotected > 0 {
            style(unprotected).red().bold()
        } else {
            style(unprotected).green().bold()
        }
    );
}

fn print_access(audits: &[RepoAudit]) {
    use tabled::builder::Builder;

    let mut builder = Builder::default();
    builder.push_record(["Repository", "Team", "Permission"]);

    for a in audits {
        let Some(access) = &a.access else {
            continue;
        };
        if access.teams.is_empty() {
            builder.push_record([
                a.name.clone(),
                style("(none)").dim().to_string(),
                String::new(),
            ]);
            continue;
        }
        for (index, team) in access.teams.iter().enumerate() {
            builder.push_record([
                if index == 0 {
                    a.name.clone()
                } else {
                    String::new()
                },
                team.slug.clone(),
                team.permission.clone(),
            ]);
        }
    }

    print_heading("Access");
    println!();
    output::print_table(builder);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tabled::builder::Builder;
    use tabled::settings::object::Columns;
    use tabled::settings::{Alignment, Modify, Style};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{AuditCategory, Sections, get_alert_counts};
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
    fn audit_format_accepts_text_table_and_json() {
        use clap::Parser;
        for (value, json) in [("text", false), ("table", false), ("json", true)] {
            let cli = crate::cli::Cli::parse_from(["ward", "audit", "--format", value]);
            let crate::cli::Command::Audit(command) = cli.command else {
                panic!("expected audit command");
            };
            assert_eq!(command.output.is_json(), json, "{value}");
        }
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

        let audit = super::audit_repo(&client, &repo, None, Sections::select(&[]))
            .await
            .unwrap();
        let joined = audit.unavailable.join("\n");
        assert!(joined.contains("rulesets:"), "{joined}");
        assert!(joined.contains("dependabot alerts:"), "{joined}");
        assert!(audit.security.unwrap().has_dependabot_config);
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

    fn repository() -> crate::github::repos::Repository {
        serde_json::from_value(json!({
            "name": "my-repo",
            "full_name": "test-org/my-repo",
            "archived": false,
            "default_branch": "main",
            "visibility": "private"
        }))
        .unwrap()
    }

    async fn mount_json(server: &MockServer, route: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
    }

    #[test]
    fn no_category_selects_every_section() {
        let sections = Sections::select(&[]);
        assert!(sections.security && sections.rulesets);
        assert!(sections.branch_protection && sections.access);
    }

    #[test]
    fn category_filter_selects_only_the_named_sections() {
        let sections = Sections::select(&[AuditCategory::Access, AuditCategory::Rulesets]);
        assert!(!sections.security && !sections.branch_protection);
        assert!(sections.access && sections.rulesets);
    }

    #[test]
    fn audit_categories_parse_with_aliases() {
        use clap::Parser;
        let cli = crate::cli::Cli::parse_from([
            "ward",
            "audit",
            "--category",
            "teams,protection,ruleset,security",
        ]);
        let crate::cli::Command::Audit(command) = cli.command else {
            panic!("expected audit command");
        };
        assert_eq!(
            command.category,
            [
                AuditCategory::Access,
                AuditCategory::BranchProtection,
                AuditCategory::Rulesets,
                AuditCategory::Security
            ]
        );
    }

    #[test]
    fn audit_rejects_a_category_it_has_no_section_for() {
        use clap::Parser;
        assert!(crate::cli::Cli::try_parse_from(["ward", "audit", "--category", "files"]).is_err());
    }

    #[tokio::test]
    async fn rulesets_section_lists_names_and_detects_copilot_review_by_rule_type() {
        let server = MockServer::start().await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/rulesets",
            200,
            json!([{"id": 1, "name": "Review policy", "target": "branch", "enforcement": "active"}]),
        )
        .await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/rules/branches/main",
            200,
            json!([{"type": "pull_request"}, {"type": "copilot_code_review"}]),
        )
        .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let sections = Sections::select(&[AuditCategory::Rulesets]);
        let audit = super::audit_repo(&client, &repository(), None, sections)
            .await
            .unwrap();
        let value = serde_json::to_value(&audit).unwrap();

        assert_eq!(value["rulesets"][0]["name"], "Review policy");
        assert_eq!(value["rulesets"][0]["enforcement"], "active");
        assert_eq!(value["settings"]["has_copilot_review_ruleset"], true);
        assert!(value.get("security").is_none());
        assert!(value.get("branch_protection").is_none());
        assert!(value.get("access").is_none());
    }

    #[tokio::test]
    async fn copilot_review_falls_back_to_the_ruleset_name_when_rules_are_unreadable() {
        let server = MockServer::start().await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/rulesets",
            200,
            json!([{"id": 1, "name": "Copilot Code Review", "target": "branch", "enforcement": "active"}]),
        )
        .await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/rules/branches/main",
            403,
            json!({"message": "Forbidden"}),
        )
        .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let sections = Sections::select(&[AuditCategory::Rulesets]);
        let audit = super::audit_repo(&client, &repository(), None, sections)
            .await
            .unwrap();

        assert!(audit.settings.unwrap().has_copilot_review_ruleset);
    }

    #[tokio::test]
    async fn rule_type_wins_over_a_misleading_ruleset_name() {
        let server = MockServer::start().await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/rulesets",
            200,
            json!([{"id": 1, "name": "Copilot Code Review", "target": "branch", "enforcement": "disabled"}]),
        )
        .await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/rules/branches/main",
            200,
            json!([]),
        )
        .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let sections = Sections::select(&[AuditCategory::Rulesets]);
        let audit = super::audit_repo(&client, &repository(), None, sections)
            .await
            .unwrap();

        assert!(!audit.settings.unwrap().has_copilot_review_ruleset);
    }

    #[tokio::test]
    async fn branch_protection_and_access_sections_report_their_fields() {
        let server = MockServer::start().await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/branches/main/protection",
            200,
            json!({
                "required_pull_request_reviews": {
                    "required_approving_review_count": 2,
                    "dismiss_stale_reviews": true,
                    "require_code_owner_reviews": false
                },
                "enforce_admins": {"enabled": true},
                "required_linear_history": {"enabled": true},
                "allow_force_pushes": {"enabled": false}
            }),
        )
        .await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/teams",
            200,
            json!([{"id": 7, "name": "Core", "slug": "core", "permission": "push", "privacy": "closed"}]),
        )
        .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let sections = Sections::select(&[AuditCategory::BranchProtection, AuditCategory::Access]);
        let audit = super::audit_repo(&client, &repository(), None, sections)
            .await
            .unwrap();
        let value = serde_json::to_value(&audit).unwrap();

        assert_eq!(value["branch_protection"]["branch"], "main");
        assert_eq!(value["branch_protection"]["protected"], true);
        assert_eq!(
            value["branch_protection"]["required_approving_review_count"],
            2
        );
        assert_eq!(value["branch_protection"]["enforce_admins"], true);
        assert_eq!(value["access"]["teams"][0]["slug"], "core");
        assert_eq!(value["access"]["teams"][0]["permission"], "push");
        assert!(audit.unavailable.is_empty(), "{:?}", audit.unavailable);
    }

    #[tokio::test]
    async fn unprotected_branch_and_unreadable_teams_do_not_abort_the_audit() {
        let server = MockServer::start().await;
        mount_json(
            &server,
            "/repos/test-org/my-repo/teams",
            403,
            json!({"message": "Forbidden"}),
        )
        .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let sections = Sections::select(&[AuditCategory::BranchProtection, AuditCategory::Access]);
        let audit = super::audit_repo(&client, &repository(), None, sections)
            .await
            .unwrap();

        let protection = audit.branch_protection.unwrap();
        assert!(!protection.protected);
        assert!(audit.access.is_none());
        assert!(audit.unavailable.join("\n").contains("teams:"));
    }

    #[tokio::test]
    async fn json_report_keeps_the_existing_keys_for_the_default_sections() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let audit = super::audit_repo(&client, &repository(), None, Sections::select(&[]))
            .await
            .unwrap();
        let value = serde_json::to_value(&audit).unwrap();

        for key in [
            "security",
            "dependency_graph",
            "settings",
            "rulesets",
            "branch_protection",
        ] {
            assert!(value.get(key).is_some(), "{key}");
        }
        assert!(value["security"].get("alert_counts").is_some());
        assert!(
            value["security"]
                .get("dependabot_security_updates")
                .is_some()
        );
    }
}
