use anyhow::Result;
use clap::Args;
use console::style;
use serde::{Deserialize, Serialize};

use crate::config::Manifest;
use crate::github::Client;
use crate::github::branch_protection::BranchProtectionState;
use crate::github::security::SecurityState;

#[derive(Args)]
pub struct PolicyCommand {
    #[command(subcommand)]
    action: PolicyAction,
}

#[derive(clap::Subcommand)]
enum PolicyAction {
    /// Check all repos against policies
    Check,

    /// List configured policies
    List,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PolicyRule {
    pub name: String,
    pub rule: String,
    #[serde(default = "default_error")]
    pub severity: PolicySeverity,
}

fn default_error() -> PolicySeverity {
    PolicySeverity::Error
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicySeverity {
    Error,
    Warning,
}

impl std::fmt::Display for PolicySeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicySeverity::Error => write!(f, "error"),
            PolicySeverity::Warning => write!(f, "warning"),
        }
    }
}

#[derive(Debug)]
struct RepoContext {
    visibility: String,
    archived: bool,
    security: SecurityState,
    branch_protection: BranchProtectionState,
}

#[derive(Debug, Serialize)]
struct Violation {
    repo: String,
    policy: String,
    severity: String,
    rule: String,
}

#[derive(Debug)]
enum ParsedRule {
    BoolField {
        path: Vec<String>,
        negated: bool,
    },
    Comparison {
        path: Vec<String>,
        op: CmpOp,
        value: CmpValue,
    },
}

#[derive(Debug)]
enum CmpOp {
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
}

#[derive(Debug)]
enum CmpValue {
    Number(f64),
    Str(String),
}

impl PolicyCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
        json: bool,
    ) -> Result<()> {
        match &self.action {
            PolicyAction::Check => check(client, manifest, system, repo, json).await,
            PolicyAction::List => list(manifest, json),
        }
    }
}

fn list(manifest: &Manifest, json: bool) -> Result<()> {
    if manifest.policies.is_empty() {
        println!("\n  No policies configured in ward.toml");
        return Ok(());
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest.policies).unwrap_or_default()
        );
        return Ok(());
    }

    println!();
    println!("  {}", style("Configured Policies").bold().cyan());
    println!("  {}", style("-".repeat(60)).dim());

    for p in &manifest.policies {
        let sev = match p.severity {
            PolicySeverity::Error => style("error").red().bold(),
            PolicySeverity::Warning => style("warning").yellow(),
        };
        println!(
            "  {} [{}] {}",
            style(&p.name).bold(),
            sev,
            style(&p.rule).dim()
        );
    }

    Ok(())
}

async fn check(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    json: bool,
) -> Result<()> {
    if manifest.policies.is_empty() {
        anyhow::bail!("No policies configured in ward.toml. Add [[policies]] entries first.");
    }

    let repos = resolve_repos(client, manifest, system, repo).await?;

    if !json {
        println!(
            "\n  {} Checking {} repos against {} policies...",
            style("[..]").dim(),
            repos.len(),
            manifest.policies.len()
        );
    }

    let mut violations = Vec::new();

    for repo_info in &repos {
        let (sec_result, prot_result) = tokio::join!(
            client.get_security_state(&repo_info.name),
            client.get_branch_protection(&repo_info.name, &repo_info.default_branch)
        );

        let ctx = RepoContext {
            visibility: repo_info.visibility.clone(),
            archived: repo_info.archived,
            security: sec_result.unwrap_or_default(),
            branch_protection: prot_result.unwrap_or(None).unwrap_or_default(),
        };

        for policy in &manifest.policies {
            match parse_rule(&policy.rule) {
                Ok(parsed) => {
                    if !evaluate_rule(&parsed, &ctx) {
                        violations.push(Violation {
                            repo: repo_info.name.clone(),
                            policy: policy.name.clone(),
                            severity: policy.severity.to_string(),
                            rule: policy.rule.clone(),
                        });
                    }
                }
                Err(e) => {
                    if !json {
                        println!(
                            "  {} Skipping policy '{}': {}",
                            style("[!!]").yellow(),
                            policy.name,
                            e
                        );
                    }
                }
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&violations).unwrap_or_default()
        );
    } else {
        print_violations(&violations);
    }

    let error_count = violations.iter().filter(|v| v.severity == "error").count();
    if error_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}

async fn resolve_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<crate::github::repos::Repository>> {
    if let Some(repo_name) = repo {
        let r = client.get_repo(repo_name).await?;
        return Ok(vec![r]);
    }

    if let Some(sys) = system {
        let excludes = manifest.exclude_patterns_for_system(sys);
        let explicit = manifest.explicit_repos_for_system(sys);
        return client
            .list_repos_for_system(sys, &excludes, &explicit)
            .await;
    }

    client.list_repos().await
}

fn print_violations(violations: &[Violation]) {
    if violations.is_empty() {
        println!(
            "\n  {} All repos comply with all policies.",
            style("[ok]").green()
        );
        return;
    }

    println!();

    let mut current_repo = "";
    for v in violations {
        if v.repo != current_repo {
            current_repo = &v.repo;
            println!("  {}", style(&v.repo).bold());
        }

        let sev = if v.severity == "error" {
            style(&v.severity).red()
        } else {
            style(&v.severity).yellow()
        };

        println!(
            "    {} [{}] {} ({})",
            style("-").dim(),
            sev,
            v.policy,
            style(&v.rule).dim()
        );
    }

    let errors = violations.iter().filter(|v| v.severity == "error").count();
    let warnings = violations
        .iter()
        .filter(|v| v.severity == "warning")
        .count();

    println!();
    println!(
        "  Summary: {} errors, {} warnings",
        if errors > 0 {
            style(errors).red().bold()
        } else {
            style(errors).green().bold()
        },
        if warnings > 0 {
            style(warnings).yellow().bold()
        } else {
            style(warnings).green().bold()
        }
    );
}

fn parse_rule(rule: &str) -> Result<ParsedRule> {
    let rule = rule.trim();

    // Negated boolean: !field.subfield
    if let Some(rest) = rule.strip_prefix('!') {
        let path = parse_path(rest.trim())?;
        return Ok(ParsedRule::BoolField {
            path,
            negated: true,
        });
    }

    // Comparison operators: >=, <=, !=, ==, >, <
    let ops = [">=", "<=", "!=", "==", ">", "<"];
    for op_str in ops {
        if let Some(pos) = rule.find(op_str) {
            let lhs = rule[..pos].trim();
            let rhs = rule[pos + op_str.len()..].trim();
            let path = parse_path(lhs)?;
            let op = match op_str {
                ">=" => CmpOp::Ge,
                "<=" => CmpOp::Le,
                "!=" => CmpOp::Ne,
                "==" => CmpOp::Eq,
                ">" => CmpOp::Gt,
                "<" => CmpOp::Lt,
                _ => unreachable!(),
            };
            let value = parse_value(rhs)?;
            return Ok(ParsedRule::Comparison { path, op, value });
        }
    }

    // Simple boolean: field.subfield
    let path = parse_path(rule)?;
    Ok(ParsedRule::BoolField {
        path,
        negated: false,
    })
}

fn parse_path(s: &str) -> Result<Vec<String>> {
    let parts: Vec<String> = s.split('.').map(|p| p.trim().to_string()).collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        anyhow::bail!("Invalid field path: {s}");
    }
    Ok(parts)
}

fn parse_value(s: &str) -> Result<CmpValue> {
    let s = s.trim();
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        return Ok(CmpValue::Str(s[1..s.len() - 1].to_string()));
    }
    if let Ok(n) = s.parse::<f64>() {
        return Ok(CmpValue::Number(n));
    }
    anyhow::bail!("Cannot parse value: {s}")
}

fn evaluate_rule(rule: &ParsedRule, ctx: &RepoContext) -> bool {
    match rule {
        ParsedRule::BoolField { path, negated } => {
            let val = resolve_bool(path, ctx);
            if *negated { !val } else { val }
        }
        ParsedRule::Comparison { path, op, value } => match value {
            CmpValue::Number(expected) => {
                let actual = resolve_number(path, ctx);
                match op {
                    CmpOp::Ge => actual >= *expected,
                    CmpOp::Le => actual <= *expected,
                    CmpOp::Gt => actual > *expected,
                    CmpOp::Lt => actual < *expected,
                    CmpOp::Eq => (actual - expected).abs() < f64::EPSILON,
                    CmpOp::Ne => (actual - expected).abs() >= f64::EPSILON,
                }
            }
            CmpValue::Str(expected) => {
                let actual = resolve_string(path, ctx);
                match op {
                    CmpOp::Eq => actual == *expected,
                    CmpOp::Ne => actual != *expected,
                    _ => false,
                }
            }
        },
    }
}

fn resolve_bool(path: &[String], ctx: &RepoContext) -> bool {
    match path.first().map(String::as_str) {
        Some("security") => match path.get(1).map(String::as_str) {
            Some("secret_scanning") => ctx.security.secret_scanning,
            Some("push_protection") => ctx.security.push_protection,
            Some("dependabot_alerts") => ctx.security.dependabot_alerts,
            Some("dependabot_security_updates") => ctx.security.dependabot_security_updates,
            Some("secret_scanning_ai_detection") => ctx.security.secret_scanning_ai_detection,
            _ => false,
        },
        Some("branch_protection") => match path.get(1).map(String::as_str) {
            Some("enabled") => ctx.branch_protection.required_pull_request_reviews,
            Some("dismiss_stale_reviews") => ctx.branch_protection.dismiss_stale_reviews,
            Some("require_code_owner_reviews") => ctx.branch_protection.require_code_owner_reviews,
            Some("require_status_checks") => ctx.branch_protection.required_status_checks,
            Some("strict_status_checks") => ctx.branch_protection.strict_status_checks,
            Some("enforce_admins") => ctx.branch_protection.enforce_admins,
            Some("required_linear_history") => ctx.branch_protection.required_linear_history,
            Some("allow_force_pushes") => ctx.branch_protection.allow_force_pushes,
            Some("allow_deletions") => ctx.branch_protection.allow_deletions,
            _ => false,
        },
        Some("archived") => ctx.archived,
        _ => false,
    }
}

fn resolve_number(path: &[String], ctx: &RepoContext) -> f64 {
    match path.first().map(String::as_str) {
        Some("branch_protection") => match path.get(1).map(String::as_str) {
            Some("required_approvals") => {
                ctx.branch_protection.required_approving_review_count as f64
            }
            _ => 0.0,
        },
        _ => 0.0,
    }
}

fn resolve_string(path: &[String], ctx: &RepoContext) -> String {
    match path.first().map(String::as_str) {
        Some("visibility") => ctx.visibility.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> RepoContext {
        RepoContext {
            visibility: "private".to_string(),
            archived: false,
            security: SecurityState {
                secret_scanning: true,
                push_protection: false,
                dependabot_alerts: true,
                dependabot_security_updates: true,
                secret_scanning_ai_detection: false,
            },
            branch_protection: BranchProtectionState {
                required_pull_request_reviews: true,
                required_approving_review_count: 2,
                dismiss_stale_reviews: true,
                require_code_owner_reviews: false,
                required_status_checks: true,
                strict_status_checks: false,
                enforce_admins: false,
                required_linear_history: false,
                allow_force_pushes: true,
                allow_deletions: false,
            },
        }
    }

    #[test]
    fn test_parse_boolean_rule() {
        let parsed = parse_rule("security.secret_scanning").unwrap();
        match parsed {
            ParsedRule::BoolField { path, negated } => {
                assert_eq!(path, vec!["security", "secret_scanning"]);
                assert!(!negated);
            }
            _ => panic!("expected BoolField"),
        }
    }

    #[test]
    fn test_parse_negated_rule() {
        let parsed = parse_rule("!branch_protection.allow_force_pushes").unwrap();
        match parsed {
            ParsedRule::BoolField { path, negated } => {
                assert_eq!(path, vec!["branch_protection", "allow_force_pushes"]);
                assert!(negated);
            }
            _ => panic!("expected negated BoolField"),
        }
    }

    #[test]
    fn test_parse_comparison_rule() {
        let parsed = parse_rule("branch_protection.required_approvals >= 2").unwrap();
        match parsed {
            ParsedRule::Comparison { path, op, value } => {
                assert_eq!(path, vec!["branch_protection", "required_approvals"]);
                assert!(matches!(op, CmpOp::Ge));
                assert!(matches!(value, CmpValue::Number(n) if (n - 2.0).abs() < f64::EPSILON));
            }
            _ => panic!("expected Comparison"),
        }
    }

    #[test]
    fn test_parse_string_rule() {
        let parsed = parse_rule("visibility != 'public'").unwrap();
        match parsed {
            ParsedRule::Comparison { path, op, value } => {
                assert_eq!(path, vec!["visibility"]);
                assert!(matches!(op, CmpOp::Ne));
                assert!(matches!(value, CmpValue::Str(ref s) if s == "public"));
            }
            _ => panic!("expected Comparison"),
        }
    }

    #[test]
    fn test_evaluate_policy_pass() {
        let ctx = make_ctx();

        // security.secret_scanning is true -- should pass
        let rule = parse_rule("security.secret_scanning").unwrap();
        assert!(evaluate_rule(&rule, &ctx));

        // visibility != 'public' -- we are 'private' -- should pass
        let rule = parse_rule("visibility != 'public'").unwrap();
        assert!(evaluate_rule(&rule, &ctx));

        // required_approvals >= 2 -- we have 2 -- should pass
        let rule = parse_rule("branch_protection.required_approvals >= 2").unwrap();
        assert!(evaluate_rule(&rule, &ctx));
    }

    #[test]
    fn test_evaluate_policy_fail() {
        let ctx = make_ctx();

        // push_protection is false -- should fail
        let rule = parse_rule("security.push_protection").unwrap();
        assert!(!evaluate_rule(&rule, &ctx));

        // !allow_force_pushes -- allow_force_pushes is true, so negation is false -- should fail
        let rule = parse_rule("!branch_protection.allow_force_pushes").unwrap();
        assert!(!evaluate_rule(&rule, &ctx));

        // required_approvals >= 3 -- we have 2 -- should fail
        let rule = parse_rule("branch_protection.required_approvals >= 3").unwrap();
        assert!(!evaluate_rule(&rule, &ctx));
    }

    #[test]
    fn test_parse_equality_string() {
        let parsed = parse_rule("visibility == 'private'").unwrap();
        match parsed {
            ParsedRule::Comparison { path, op, value } => {
                assert_eq!(path, vec!["visibility"]);
                assert!(matches!(op, CmpOp::Eq));
                assert!(matches!(value, CmpValue::Str(ref s) if s == "private"));
            }
            _ => panic!("expected Comparison"),
        }
    }

    #[test]
    fn test_evaluate_archived_bool() {
        let ctx = make_ctx();
        let rule = parse_rule("!archived").unwrap();
        assert!(evaluate_rule(&rule, &ctx)); // archived is false, !false = true
    }

    #[test]
    fn test_policy_rule_serde() {
        let toml_str = r#"
            name = "no-public"
            rule = "visibility != 'public'"
            severity = "error"
        "#;
        let rule: PolicyRule = toml::from_str(toml_str).unwrap();
        assert_eq!(rule.name, "no-public");
        assert_eq!(rule.severity, PolicySeverity::Error);
    }

    #[test]
    fn test_policy_severity_default() {
        let toml_str = r#"
            name = "test"
            rule = "security.secret_scanning"
        "#;
        let rule: PolicyRule = toml::from_str(toml_str).unwrap();
        assert_eq!(rule.severity, PolicySeverity::Error);
    }
}
