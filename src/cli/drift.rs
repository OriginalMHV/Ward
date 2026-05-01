use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;

use crate::config::Manifest;
use crate::config::manifest::{BranchProtectionConfig, SecurityConfig};
use crate::github::Client;
use crate::github::branch_protection::BranchProtectionState;
use crate::github::security::SecurityState;

#[derive(Args)]
pub struct DriftCommand {
    #[command(subcommand)]
    action: DriftAction,
}

#[derive(clap::Subcommand)]
enum DriftAction {
    /// Check for configuration drift across repos
    Check,
}

#[derive(Debug, Serialize)]
pub struct DriftResult {
    pub repo: String,
    pub security_drifts: Vec<DriftItem>,
    pub protection_drifts: Vec<DriftItem>,
}

#[derive(Debug, Serialize)]
pub struct DriftItem {
    pub field: String,
    pub expected: String,
    pub actual: String,
}

impl DriftResult {
    fn status(&self) -> &str {
        if self.is_drifted() { "drifted" } else { "ok" }
    }

    fn is_drifted(&self) -> bool {
        !self.security_drifts.is_empty() || !self.protection_drifts.is_empty()
    }
}

pub fn compare_security(desired: &SecurityConfig, actual: &SecurityState) -> Vec<DriftItem> {
    let mut drifts = Vec::new();

    let checks: &[(&str, bool, bool)] = &[
        (
            "secret_scanning",
            desired.secret_scanning,
            actual.secret_scanning,
        ),
        (
            "push_protection",
            desired.push_protection,
            actual.push_protection,
        ),
        (
            "dependabot_alerts",
            desired.dependabot_alerts,
            actual.dependabot_alerts,
        ),
        (
            "dependabot_security_updates",
            desired.dependabot_security_updates,
            actual.dependabot_security_updates,
        ),
        (
            "secret_scanning_ai_detection",
            desired.secret_scanning_ai_detection,
            actual.secret_scanning_ai_detection,
        ),
    ];

    for &(field, expected, actual_val) in checks {
        if expected != actual_val {
            drifts.push(DriftItem {
                field: field.to_string(),
                expected: expected.to_string(),
                actual: actual_val.to_string(),
            });
        }
    }

    drifts
}

pub fn compare_protection(
    desired: &BranchProtectionConfig,
    actual: &BranchProtectionState,
) -> Vec<DriftItem> {
    let mut drifts = Vec::new();

    let checks: &[(&str, bool, bool)] = &[
        (
            "required_approvals_enabled",
            desired.enabled,
            actual.required_pull_request_reviews,
        ),
        (
            "dismiss_stale_reviews",
            desired.dismiss_stale_reviews,
            actual.dismiss_stale_reviews,
        ),
        (
            "require_code_owner_reviews",
            desired.require_code_owner_reviews,
            actual.require_code_owner_reviews,
        ),
        (
            "require_status_checks",
            desired.require_status_checks,
            actual.required_status_checks,
        ),
        (
            "strict_status_checks",
            desired.strict_status_checks,
            actual.strict_status_checks,
        ),
        (
            "enforce_admins",
            desired.enforce_admins,
            actual.enforce_admins,
        ),
        (
            "required_linear_history",
            desired.required_linear_history,
            actual.required_linear_history,
        ),
        (
            "allow_force_pushes",
            desired.allow_force_pushes,
            actual.allow_force_pushes,
        ),
        (
            "allow_deletions",
            desired.allow_deletions,
            actual.allow_deletions,
        ),
    ];

    for &(field, expected, actual_val) in checks {
        if expected != actual_val {
            drifts.push(DriftItem {
                field: field.to_string(),
                expected: expected.to_string(),
                actual: actual_val.to_string(),
            });
        }
    }

    if desired.required_approvals != actual.required_approving_review_count {
        drifts.push(DriftItem {
            field: "required_approvals".to_string(),
            expected: desired.required_approvals.to_string(),
            actual: actual.required_approving_review_count.to_string(),
        });
    }

    drifts
}

impl DriftCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
        json: bool,
    ) -> Result<()> {
        match &self.action {
            DriftAction::Check => check(client, manifest, system, repo, json).await,
        }
    }
}

async fn resolve_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<(String, String)>> {
    if let Some(repo_name) = repo {
        let r = client.get_repo(repo_name).await?;
        return Ok(vec![(r.name, r.default_branch)]);
    }

    let sys = system.ok_or_else(|| {
        anyhow::anyhow!("Either --system or --repo is required for drift commands")
    })?;

    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(sys, &excludes, &explicit)
        .await?;
    Ok(repos
        .into_iter()
        .map(|r| (r.name, r.default_branch))
        .collect())
}

async fn check(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    json: bool,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;
    let sys_id = system.unwrap_or("default");
    let desired_security = manifest.security_for_system(sys_id);
    let desired_protection = &manifest.branch_protection;

    if !json {
        println!();
        println!(
            "  {} Checking drift for {} repositories...",
            style("[..]").dim(),
            repos.len()
        );
    }

    let mut results = Vec::new();

    for (repo_name, default_branch) in &repos {
        let (security_result, protection_result) = tokio::join!(
            client.get_security_state(repo_name),
            client.get_branch_protection(repo_name, default_branch)
        );

        let security_state = security_result?;
        let protection_state = protection_result?.unwrap_or_default();

        let security_drifts = compare_security(desired_security, &security_state);
        let protection_drifts = compare_protection(desired_protection, &protection_state);

        results.push(DriftResult {
            repo: repo_name.clone(),
            security_drifts,
            protection_drifts,
        });
    }

    if json {
        print_json(&results);
    } else {
        print_table(&results);
    }

    let drifted = results.iter().filter(|r| r.is_drifted()).count();
    if drifted > 0 {
        anyhow::bail!(
            "{drifted} {} with configuration drift",
            if drifted == 1 {
                "repository"
            } else {
                "repositories"
            }
        );
    }

    Ok(())
}

fn print_json(results: &[DriftResult]) {
    #[derive(Serialize)]
    struct JsonEntry<'a> {
        repo: &'a str,
        security_drift: &'a [DriftItem],
        protection_drift: &'a [DriftItem],
        status: &'a str,
    }

    let output: Vec<JsonEntry<'_>> = results
        .iter()
        .map(|r| JsonEntry {
            repo: &r.repo,
            security_drift: &r.security_drifts,
            protection_drift: &r.protection_drifts,
            status: r.status(),
        })
        .collect();

    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_default()
    );
}

fn print_table(results: &[DriftResult]) {
    println!();
    println!(
        "  {} {} {} {}",
        style(format!("{:<40}", "Repository")).bold().underlined(),
        style(format!("{:<15}", "Security")).bold().underlined(),
        style(format!("{:<15}", "Protection")).bold().underlined(),
        style("Status").bold().underlined(),
    );
    println!("  {}", style("\u{2500}".repeat(80)).dim());

    for result in results {
        let sec = if result.security_drifts.is_empty() {
            format!("{}", style(format!("{:<15}", "[ok]")).green())
        } else {
            format!("{}", style(format!("{:<15}", "[!!]")).red())
        };
        let prot = if result.protection_drifts.is_empty() {
            format!("{}", style(format!("{:<15}", "[ok]")).green())
        } else {
            format!("{}", style(format!("{:<15}", "[!!]")).red())
        };
        let status = if result.is_drifted() {
            format!("{}", style("DRIFTED").red().bold())
        } else {
            format!("{}", style("In sync").green())
        };

        println!("  {:<40} {} {} {}", result.repo, sec, prot, status);

        for drift in &result.security_drifts {
            println!(
                "    - {}: expected {}, got {}",
                drift.field,
                style(&drift.expected).green(),
                style(&drift.actual).red()
            );
        }
        for drift in &result.protection_drifts {
            println!(
                "    - {}: expected {}, got {}",
                drift.field,
                style(&drift.expected).green(),
                style(&drift.actual).red()
            );
        }
    }

    let total = results.len();
    let in_sync = results.iter().filter(|r| !r.is_drifted()).count();
    let drifted = total - in_sync;

    println!();
    println!(
        "  Summary: {}/{} in sync, {}/{} drifted",
        style(in_sync).green().bold(),
        total,
        if drifted > 0 {
            style(drifted).red().bold()
        } else {
            style(drifted).green().bold()
        },
        total,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_drift_detection() {
        let desired = SecurityConfig {
            secret_scanning: true,
            push_protection: true,
            dependabot_alerts: true,
            dependabot_security_updates: true,
            secret_scanning_ai_detection: true,
            codeql_advanced_setup: false,
            checks: vec![],
        };
        let actual = SecurityState {
            secret_scanning: false,
            push_protection: false,
            dependabot_alerts: true,
            dependabot_security_updates: true,
            secret_scanning_ai_detection: true,
        };

        let drifts = compare_security(&desired, &actual);
        assert_eq!(drifts.len(), 2);
        assert_eq!(drifts[0].field, "secret_scanning");
        assert_eq!(drifts[0].expected, "true");
        assert_eq!(drifts[0].actual, "false");
        assert_eq!(drifts[1].field, "push_protection");
    }

    #[test]
    fn test_protection_drift_detection() {
        let desired = BranchProtectionConfig {
            enabled: true,
            required_approvals: 1,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            require_status_checks: false,
            strict_status_checks: false,
            enforce_admins: false,
            required_linear_history: false,
            allow_force_pushes: false,
            allow_deletions: false,
        };
        let actual = BranchProtectionState {
            required_pull_request_reviews: true,
            required_approving_review_count: 0,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            required_status_checks: false,
            strict_status_checks: false,
            enforce_admins: false,
            required_linear_history: false,
            allow_force_pushes: false,
            allow_deletions: false,
        };

        let drifts = compare_protection(&desired, &actual);
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].field, "required_approvals");
        assert_eq!(drifts[0].expected, "1");
        assert_eq!(drifts[0].actual, "0");
    }

    #[test]
    fn test_no_drift_returns_empty() {
        let desired_sec = SecurityConfig {
            secret_scanning: true,
            push_protection: true,
            dependabot_alerts: true,
            dependabot_security_updates: true,
            secret_scanning_ai_detection: true,
            codeql_advanced_setup: false,
            checks: vec![],
        };
        let actual_sec = SecurityState {
            secret_scanning: true,
            push_protection: true,
            dependabot_alerts: true,
            dependabot_security_updates: true,
            secret_scanning_ai_detection: true,
        };

        let desired_prot = BranchProtectionConfig {
            enabled: false,
            required_approvals: 0,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            require_status_checks: false,
            strict_status_checks: false,
            enforce_admins: false,
            required_linear_history: false,
            allow_force_pushes: false,
            allow_deletions: false,
        };
        let actual_prot = BranchProtectionState::default();

        assert!(compare_security(&desired_sec, &actual_sec).is_empty());
        assert!(compare_protection(&desired_prot, &actual_prot).is_empty());
    }

    #[test]
    fn test_drift_item_formatting() {
        let item = DriftItem {
            field: "secret_scanning".to_string(),
            expected: "true".to_string(),
            actual: "false".to_string(),
        };
        assert_eq!(item.field, "secret_scanning");
        assert_eq!(item.expected, "true");
        assert_eq!(item.actual, "false");

        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("secret_scanning"));
        assert!(json.contains(r#""expected":"true""#));
        assert!(json.contains(r#""actual":"false""#));
    }
}
