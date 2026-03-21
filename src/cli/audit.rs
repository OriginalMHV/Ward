use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;

use crate::config::Manifest;
use crate::detection::versions;
use crate::github::Client;

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
    system_id: String,
    project_type: String,
    language: Option<String>,
    description: Option<String>,
    default_branch: String,
    versions: VersionInfo,
    security: SecurityAudit,
    settings: SettingsAudit,
}

#[derive(Debug, Default, Serialize)]
struct VersionInfo {
    java: Option<String>,
    node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spring_boot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kotlin: Option<String>,
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
    has_dependency_submission: bool,
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
    ) -> Result<()> {
        let sys = system.ok_or_else(|| anyhow::anyhow!("--system is required for audit"))?;

        let excludes = manifest.exclude_patterns_for_system(sys);
        let repos = client.list_repos_for_system(sys, &excludes).await?;

        println!();
        println!(
            "  {} Full audit — {} repos in system {}",
            style("🔍").bold(),
            repos.len(),
            style(sys).cyan()
        );

        let mut audits = Vec::new();

        for repo in &repos {
            tracing::info!("Auditing {}...", repo.name);
            let audit = audit_repo(client, &repo.name, sys).await?;
            audits.push(audit);
        }

        if self.format == "json" {
            let report = AuditReport {
                generated_at: chrono::Utc::now().to_rfc3339(),
                organization: client.org.clone(),
                repositories: audits,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print_table(&audits);
        }

        Ok(())
    }
}

async fn audit_repo(client: &Client, repo: &str, system_id: &str) -> Result<RepoAudit> {
    let repo_info = client.get_repo(repo).await?;
    let security_state = client.get_security_state(repo).await?;

    // Detect project type and versions
    let mut project_type = "unknown".to_owned();
    let mut version_info = VersionInfo::default();

    if let Some(content) = client.get_file(repo, "build.gradle.kts", None).await? {
        project_type = "gradle".to_owned();
        let text = Client::decode_content(&content).unwrap_or_default();
        if let Some(v) = versions::extract_java_version(&text) {
            version_info.java = Some(v.to_string());
        }
        // Try to detect Spring Boot version
        version_info.spring_boot = extract_spring_boot_version(&text);
        if text.contains("kotlin") {
            version_info.kotlin = Some("detected".to_owned());
        }
    } else if let Some(content) = client.get_file(repo, "build.gradle", None).await? {
        project_type = "gradle".to_owned();
        let text = Client::decode_content(&content).unwrap_or_default();
        if let Some(v) = versions::extract_java_version(&text) {
            version_info.java = Some(v.to_string());
        }
        version_info.spring_boot = extract_spring_boot_version(&text);
    } else if let Some(content) = client.get_file(repo, "package.json", None).await? {
        project_type = "npm".to_owned();
        let text = Client::decode_content(&content).unwrap_or_default();
        version_info.node = versions::extract_node_version(&text);
    }

    // Check for config files
    let has_dependabot_config = client
        .get_file(repo, ".github/dependabot.yml", None)
        .await?
        .is_some();
    let has_codeql = client
        .get_file(repo, ".github/workflows/codeql.yml", None)
        .await?
        .is_some();
    let has_dependency_submission = client
        .get_file(repo, ".github/workflows/dependency-submission.yml", None)
        .await?
        .is_some();

    // Check rulesets and instructions
    let rulesets = client.list_rulesets(repo).await.unwrap_or_default();
    let has_copilot_review = rulesets.iter().any(|r| r.name == "Copilot Code Review");
    let has_copilot_instructions = client
        .get_file(repo, ".github/copilot-instructions.md", None)
        .await?
        .is_some();

    // Get alert counts
    let alert_counts = get_alert_counts(client, repo).await.unwrap_or_default();

    Ok(RepoAudit {
        name: repo.to_owned(),
        system_id: system_id.to_owned(),
        project_type,
        language: repo_info.language,
        description: repo_info.description,
        default_branch: repo_info.default_branch,
        versions: version_info,
        security: SecurityAudit {
            dependabot_alerts: security_state.dependabot_alerts,
            dependabot_security_updates: security_state.dependabot_security_updates,
            secret_scanning: security_state.secret_scanning,
            secret_scanning_ai: security_state.secret_scanning_ai_detection,
            push_protection: security_state.push_protection,
            has_dependabot_config,
            has_codeql,
            has_dependency_submission,
            alert_counts,
        },
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
                client.org
            ))
            .await?;

        if resp.status().is_success() {
            // Use the array length or a header if available
            let alerts: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
            // This gives us a rough count (limited by per_page, but indicates presence)
            // For exact counts, we'd need to paginate, but this is fast enough for audit
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

    // Re-fetch with higher limit for a more accurate count
    let resp = client
        .get(&format!(
            "/repos/{}/{repo}/dependabot/alerts?state=open&per_page=100",
            client.org
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
    // Look for Spring Boot plugin version
    for line in content.lines() {
        let trimmed = line.trim();
        // id("org.springframework.boot") version "3.5.6"
        if trimmed.contains("org.springframework.boot") && trimmed.contains("version") {
            return extract_quoted_version(trimmed);
        }
        // springBootVersion = "3.5.6"
        if trimmed.contains("springBootVersion") {
            return extract_quoted_version(trimmed);
        }
    }
    None
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
        "  {:35} {:7} {:6} {:6} {:5} {:5} {:5} {:5} {:5} {:5} {:5}",
        style("Repository").bold().underlined(),
        style("Type").bold().underlined(),
        style("Java").bold().underlined(),
        style("SBoot").bold().underlined(),
        style("Dep.A").bold().underlined(),
        style("SecSc").bold().underlined(),
        style("Push").bold().underlined(),
        style("DBot").bold().underlined(),
        style("CQL").bold().underlined(),
        style("CopRv").bold().underlined(),
        style("Alert").bold().underlined(),
    );

    let mut total_alerts = 0u32;
    let mut fully_secured = 0;

    for a in audits {
        let java = a.versions.java.as_deref().unwrap_or("-");
        let sboot = a.versions.spring_boot.as_deref().unwrap_or("-");

        let icon = |b: bool| {
            if b {
                format!("{}", style("✅").green())
            } else {
                format!("{}", style("❌").red())
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

        println!(
            "  {:35} {:7} {:6} {:6} {:5} {:5} {:5} {:5} {:5} {:5} {:5}",
            &a.name,
            &a.project_type,
            java,
            sboot,
            icon(a.security.dependabot_alerts),
            icon(a.security.secret_scanning),
            icon(a.security.push_protection),
            icon(a.security.has_dependabot_config),
            icon(a.security.has_codeql),
            icon(a.settings.has_copilot_review_ruleset),
            alert_str,
        );
    }

    println!();
    println!(
        "  {} repos audited | {} fully secured | {} total open alerts",
        style(audits.len()).bold(),
        style(fully_secured).green().bold(),
        if total_alerts > 0 {
            style(total_alerts).red().bold()
        } else {
            style(total_alerts).green().bold()
        }
    );
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
}
