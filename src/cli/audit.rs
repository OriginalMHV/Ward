use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;
use serde_json::Value;

use crate::config::Manifest;
use crate::detection::versions;
use crate::github::Client;
use crate::github::contents::DirectoryEntry;
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
    project_type: String,
    language: Option<String>,
    description: Option<String>,
    default_branch: String,
    versions: VersionInfo,
    security: SecurityAudit,
    dependency_graph: DependencyGraphAudit,
    settings: SettingsAudit,
}

#[derive(Debug, Default, Serialize)]
struct VersionInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    java: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dotnet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    go: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rust: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kotlin: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    frameworks: Vec<String>,
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
        .list_repos_for_system(sys, &excludes, &explicit)
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
    let (project_type, version_info) = detect_project_metadata(client, repo, repo_info).await?;

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
        project_type,
        language: repo_info.language.clone(),
        description: repo_info.description.clone(),
        default_branch: repo_info.default_branch.clone(),
        versions: version_info,
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

async fn detect_project_metadata(
    client: &Client,
    repo: &str,
    repo_info: &Repository,
) -> Result<(String, VersionInfo)> {
    let root_entries = client
        .list_directory(repo, "", None)
        .await?
        .unwrap_or_default();

    if has_root_entry(&root_entries, "build.gradle.kts") {
        return Ok((
            "gradle".to_owned(),
            detect_gradle_metadata(client, repo, "build.gradle.kts").await?,
        ));
    }

    if has_root_entry(&root_entries, "build.gradle") {
        return Ok((
            "gradle".to_owned(),
            detect_gradle_metadata(client, repo, "build.gradle").await?,
        ));
    }

    if has_dotnet_files(&root_entries)
        || matches!(repo_info.language.as_deref(), Some("C#") | Some("F#"))
    {
        return Ok((
            "dotnet".to_owned(),
            detect_dotnet_metadata(client, repo, &root_entries).await?,
        ));
    }

    if has_root_entry(&root_entries, "package.json") {
        return Ok(("npm".to_owned(), detect_npm_metadata(client, repo).await?));
    }

    if has_root_entry(&root_entries, "go.mod") {
        return Ok(("go".to_owned(), detect_go_metadata(client, repo).await?));
    }

    if has_root_entry(&root_entries, "Cargo.toml")
        || has_root_entry(&root_entries, "rust-toolchain.toml")
        || has_root_entry(&root_entries, "rust-toolchain")
    {
        return Ok((
            "cargo".to_owned(),
            detect_cargo_metadata(client, repo, &root_entries).await?,
        ));
    }

    Ok(("unknown".to_owned(), VersionInfo::default()))
}

fn has_root_entry(entries: &[DirectoryEntry], name: &str) -> bool {
    entries.iter().any(|entry| entry.name == name)
}

fn has_dotnet_files(entries: &[DirectoryEntry]) -> bool {
    has_root_entry(entries, "global.json")
        || entries
            .iter()
            .any(|entry| entry.name.ends_with(".csproj") || entry.name.ends_with(".sln"))
}

async fn detect_gradle_metadata(client: &Client, repo: &str, path: &str) -> Result<VersionInfo> {
    let mut version_info = VersionInfo::default();
    if let Some(content) = client.get_file(repo, path, None).await? {
        let text = Client::decode_content(&content).unwrap_or_default();
        if let Some(v) = versions::extract_java_version(&text) {
            version_info.java = Some(v.to_string());
        }
        if let Some(spring_boot) = extract_spring_boot_version(&text) {
            version_info
                .frameworks
                .push(format!("spring-boot {spring_boot}"));
        } else if text.contains("org.springframework.boot") {
            version_info.frameworks.push("spring-boot".to_owned());
        }
        if text.contains("kotlin") {
            version_info.kotlin = Some("detected".to_owned());
        }
    }
    Ok(version_info)
}

async fn detect_npm_metadata(client: &Client, repo: &str) -> Result<VersionInfo> {
    let mut version_info = VersionInfo::default();
    if let Some(content) = client.get_file(repo, "package.json", None).await? {
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.node = versions::extract_node_version(&text);
        if let Some(next_version) = extract_package_json_dependency_version(&text, "next") {
            version_info
                .frameworks
                .push(format!("next.js {next_version}"));
        }
    }
    Ok(version_info)
}

async fn detect_dotnet_metadata(
    client: &Client,
    repo: &str,
    root_entries: &[DirectoryEntry],
) -> Result<VersionInfo> {
    let mut version_info = VersionInfo::default();

    if let Some(content) = client.get_file(repo, "global.json", None).await? {
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.dotnet = extract_dotnet_sdk_version(&text);
    }

    if version_info.dotnet.is_none()
        && let Some(project_path) = root_entries
            .iter()
            .find(|entry| entry.name.ends_with(".csproj"))
            .map(|entry| entry.path.as_str())
        && let Some(content) = client.get_file(repo, project_path, None).await?
    {
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.dotnet = extract_target_framework(&text);
    }

    Ok(version_info)
}

async fn detect_go_metadata(client: &Client, repo: &str) -> Result<VersionInfo> {
    let mut version_info = VersionInfo::default();
    if let Some(content) = client.get_file(repo, "go.mod", None).await? {
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.go = extract_go_version(&text);
    }
    Ok(version_info)
}

async fn detect_cargo_metadata(
    client: &Client,
    repo: &str,
    root_entries: &[DirectoryEntry],
) -> Result<VersionInfo> {
    let mut version_info = VersionInfo::default();

    if let Some(content) = client.get_file(repo, "rust-toolchain.toml", None).await? {
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.rust = extract_rust_toolchain_version(&text);
    }

    if version_info.rust.is_none()
        && let Some(content) = client.get_file(repo, "rust-toolchain", None).await?
    {
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.rust = extract_plain_toolchain(&text);
    }

    if version_info.rust.is_none()
        && has_root_entry(root_entries, "Cargo.toml")
        && let Some(content) = client.get_file(repo, "Cargo.toml", None).await?
    {
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.rust = extract_rust_version_from_cargo(&text);
    }

    Ok(version_info)
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

fn extract_spring_boot_version(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("org.springframework.boot") && trimmed.contains("version") {
            return extract_quoted_version(trimmed);
        }
        if trimmed.contains("springBootVersion") {
            return extract_quoted_version(trimmed);
        }
    }
    None
}

fn extract_package_json_dependency_version(content: &str, package: &str) -> Option<String> {
    let package_json: Value = serde_json::from_str(content).ok()?;
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(version) = package_json
            .get(key)
            .and_then(|deps| deps.get(package))
            .and_then(Value::as_str)
        {
            return Some(version.to_owned());
        }
    }
    None
}

fn extract_dotnet_sdk_version(content: &str) -> Option<String> {
    let global_json: Value = serde_json::from_str(content).ok()?;
    global_json
        .get("sdk")
        .and_then(|sdk| sdk.get("version"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn extract_target_framework(content: &str) -> Option<String> {
    extract_xml_tag(content, "TargetFramework").or_else(|| {
        extract_xml_tag(content, "TargetFrameworks").and_then(|frameworks| {
            frameworks
                .split(';')
                .next()
                .map(str::trim)
                .map(str::to_owned)
        })
    })
}

fn extract_go_version(content: &str) -> Option<String> {
    content.lines().map(str::trim).find_map(|line| {
        line.strip_prefix("go ")
            .map(|version| version.trim().to_owned())
    })
}

fn extract_rust_toolchain_version(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("channel") {
            return extract_quoted_version(trimmed);
        }
    }
    None
}

fn extract_plain_toolchain(content: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
}

fn extract_rust_version_from_cargo(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("rust-version") {
            extract_quoted_version(trimmed)
        } else {
            None
        }
    })
}

fn extract_xml_tag(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)? + start;
    Some(content[start..end].trim().to_owned())
}

fn extract_quoted_version(s: &str) -> Option<String> {
    let mut in_quote = false;
    let mut version = String::new();
    for ch in s.chars() {
        if ch == '"' || ch == '\'' {
            if in_quote
                && !version.is_empty()
                && version.contains('.')
                && version.chars().all(|c| c.is_ascii_digit() || c == '.')
            {
                return Some(version);
            }
            in_quote = !in_quote;
            version.clear();
        } else if in_quote {
            version.push(ch);
        }
    }
    None
}

fn print_table(audits: &[RepoAudit]) {
    println!();
    println!(
        "  {:35} {:8} {:12} {:18} {:5} {:5} {:5} {:5} {:5} {:5} {:5} {:5}",
        style("Repository").bold().underlined(),
        style("Type").bold().underlined(),
        style("Runtime").bold().underlined(),
        style("Framework").bold().underlined(),
        style("Dep.A").bold().underlined(),
        style("SecSc").bold().underlined(),
        style("Push").bold().underlined(),
        style("DBot").bold().underlined(),
        style("CQL").bold().underlined(),
        style("SBOM").bold().underlined(),
        style("CopRv").bold().underlined(),
        style("Alert").bold().underlined(),
    );

    let mut total_alerts = 0u32;
    let mut fully_secured = 0;
    let mut dependency_graph_available = 0;

    for a in audits {
        let runtime = truncate_cell(&runtime_summary(&a.versions), 12);
        let framework = truncate_cell(&framework_summary(&a.versions), 18);

        let icon = |b: bool| {
            if b {
                format!("{}", style("[ok]").green())
            } else {
                format!("{}", style("[!!]").red())
            }
        };

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

        println!(
            "  {:35} {:8} {:12} {:18} {:5} {:5} {:5} {:5} {:5} {:5} {:5} {:5}",
            &a.name,
            &a.project_type,
            runtime,
            framework,
            icon(a.security.dependabot_alerts),
            icon(a.security.secret_scanning),
            icon(a.security.push_protection),
            icon(a.security.has_dependabot_config),
            icon(a.security.has_codeql),
            dependency_graph_icon,
            icon(a.settings.has_copilot_review_ruleset),
            alert_str,
        );
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

fn runtime_summary(versions: &VersionInfo) -> String {
    if let Some(java) = &versions.java {
        format!("Java {java}")
    } else if let Some(node) = &versions.node {
        format!("Node {node}")
    } else if let Some(dotnet) = &versions.dotnet {
        format!(".NET {dotnet}")
    } else if let Some(go) = &versions.go {
        format!("Go {go}")
    } else if let Some(rust) = &versions.rust {
        format!("Rust {rust}")
    } else {
        "-".to_owned()
    }
}

fn framework_summary(versions: &VersionInfo) -> String {
    if versions.frameworks.is_empty() {
        "-".to_owned()
    } else {
        versions.frameworks.join(", ")
    }
}

fn truncate_cell(value: &str, width: usize) -> String {
    if value.len() <= width {
        value.to_owned()
    } else {
        format!("{}...", &value[..width.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_spring_boot_version() {
        assert_eq!(
            extract_spring_boot_version(r#"id("org.springframework.boot") version "3.5.6""#),
            Some("3.5.6".to_owned())
        );
    }

    #[test]
    fn test_extract_spring_boot_variable() {
        assert_eq!(
            extract_spring_boot_version(r#"val springBootVersion = "3.4.1""#),
            Some("3.4.1".to_owned())
        );
    }

    #[test]
    fn test_no_spring_boot() {
        assert_eq!(
            extract_spring_boot_version("plugins { id(\"java\") }"),
            None
        );
    }

    #[test]
    fn test_extract_next_version() {
        assert_eq!(
            extract_package_json_dependency_version(
                r#"{"dependencies":{"next":"^14.2.3"},"devDependencies":{"typescript":"5.4.0"}}"#,
                "next"
            ),
            Some("^14.2.3".to_owned())
        );
    }

    #[test]
    fn test_extract_dotnet_sdk_version() {
        assert_eq!(
            extract_dotnet_sdk_version(r#"{"sdk":{"version":"8.0.204"}}"#),
            Some("8.0.204".to_owned())
        );
    }

    #[test]
    fn test_extract_target_framework() {
        assert_eq!(
            extract_target_framework(
                r#"<Project><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>"#
            ),
            Some("net8.0".to_owned())
        );
    }

    #[test]
    fn test_extract_go_version() {
        assert_eq!(
            extract_go_version("module example.com/ward\n\ngo 1.22.2\n"),
            Some("1.22.2".to_owned())
        );
    }

    #[test]
    fn test_extract_rust_toolchain_version() {
        assert_eq!(
            extract_rust_toolchain_version(
                "[toolchain]\nchannel = \"1.78.0\"\ncomponents = [\"clippy\"]"
            ),
            Some("1.78.0".to_owned())
        );
    }

    #[test]
    fn test_runtime_summary_prefers_detected_runtime() {
        let versions = VersionInfo {
            dotnet: Some("net8.0".to_owned()),
            ..VersionInfo::default()
        };
        assert_eq!(runtime_summary(&versions), ".NET net8.0");
    }

    #[test]
    fn test_framework_summary_joins_multiple_frameworks() {
        let versions = VersionInfo {
            frameworks: vec!["spring-boot 3.5.6".to_owned(), "next.js ^14.2.3".to_owned()],
            ..VersionInfo::default()
        };
        assert_eq!(
            framework_summary(&versions),
            "spring-boot 3.5.6, next.js ^14.2.3"
        );
    }
}
