use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;

use crate::cli::drift::{compare_protection, compare_security};
use crate::config::Manifest;
use crate::config::manifest::TeamAccess;
use crate::github::Client;
use crate::github::repos::Repository;

#[derive(Args)]
pub struct PlanCommand {
    /// Check all systems (default: requires --system)
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Serialize)]
struct PlanReport {
    systems: Vec<SystemPlan>,
    total_repos: usize,
    total_actions: usize,
}

#[derive(Debug, Serialize)]
struct SystemPlan {
    id: String,
    repo_count: usize,
    security: CategoryResult,
    branch_protection: CategoryResult,
    rulesets: CategoryResult,
    teams: CategoryResult,
}

#[derive(Debug, Serialize)]
struct CategoryResult {
    compliant: usize,
    total: usize,
    issues: Vec<String>,
}

impl PlanCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        json: bool,
    ) -> Result<()> {
        let system_ids = resolve_system_ids(manifest, system, self.all)?;

        let mut report = PlanReport {
            systems: Vec::new(),
            total_repos: 0,
            total_actions: 0,
        };

        for sys_id in &system_ids {
            let excludes = manifest.exclude_patterns_for_system(sys_id);
            let explicit = manifest.explicit_repos_for_system(sys_id);
            let repos = client
                .list_repos_for_system(sys_id, &excludes, &explicit)
                .await?;

            if repos.is_empty() {
                continue;
            }

            let sys_plan = check_system(client, manifest, sys_id, &repos).await?;
            report.total_repos += sys_plan.repo_count;
            report.total_actions += count_actions(&sys_plan);
            report.systems.push(sys_plan);
        }

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        } else {
            print_report(&report);
        }

        Ok(())
    }
}

fn resolve_system_ids(manifest: &Manifest, system: Option<&str>, all: bool) -> Result<Vec<String>> {
    if let Some(sys) = system {
        return Ok(vec![sys.to_string()]);
    }

    if all || !manifest.systems.is_empty() {
        let ids: Vec<String> = manifest.systems.iter().map(|s| s.id.clone()).collect();
        if ids.is_empty() {
            anyhow::bail!("No systems configured in ward.toml");
        }
        return Ok(ids);
    }

    anyhow::bail!("Use --system <ID> or --all to scan all systems")
}

async fn check_system(
    client: &Client,
    manifest: &Manifest,
    sys_id: &str,
    repos: &[Repository],
) -> Result<SystemPlan> {
    let desired_security = manifest.security_for_system(sys_id);
    let desired_protection = &manifest.branch_protection;

    let mut security_result = CategoryResult {
        compliant: 0,
        total: repos.len(),
        issues: Vec::new(),
    };
    let mut protection_result = CategoryResult {
        compliant: 0,
        total: repos.len(),
        issues: Vec::new(),
    };
    let mut rulesets_result = CategoryResult {
        compliant: 0,
        total: repos.len(),
        issues: Vec::new(),
    };
    let mut teams_result = CategoryResult {
        compliant: 0,
        total: repos.len(),
        issues: Vec::new(),
    };

    let desired_teams: &[TeamAccess] = manifest
        .system(sys_id)
        .map(|s| s.teams.as_slice())
        .unwrap_or(&[]);

    let expected_ruleset = manifest.rulesets.branch_protection.as_ref().and_then(|c| {
        if c.enabled {
            Some(c.name.as_deref().unwrap_or("Branch Protection"))
        } else {
            None
        }
    });

    for repo in repos {
        // Security
        if let Ok(sec_state) = client.get_security_state(&repo.name).await {
            let drifts = compare_security(desired_security, &sec_state);
            if drifts.is_empty() {
                security_result.compliant += 1;
            } else {
                security_result.issues.push(repo.name.clone());
            }
        }

        // Branch protection
        if let Ok(prot_opt) = client
            .get_branch_protection(&repo.name, &repo.default_branch)
            .await
        {
            let prot_state = prot_opt.unwrap_or_default();
            let drifts = compare_protection(desired_protection, &prot_state);
            if drifts.is_empty() {
                protection_result.compliant += 1;
            } else {
                protection_result.issues.push(repo.name.clone());
            }
        }

        // Rulesets
        if let Some(expected_name) = expected_ruleset {
            if let Ok(rulesets) = client.list_rulesets(&repo.name).await {
                if rulesets.iter().any(|r| r.name == expected_name) {
                    rulesets_result.compliant += 1;
                } else {
                    rulesets_result.issues.push(repo.name.clone());
                }
            }
        } else {
            rulesets_result.compliant += 1;
        }

        // Teams
        if desired_teams.is_empty() {
            teams_result.compliant += 1;
        } else if let Ok(current_teams) = client.list_repo_teams(&repo.name).await {
            let all_present = desired_teams.iter().all(|d| {
                current_teams
                    .iter()
                    .any(|t| t.slug == d.slug && t.permission == d.permission)
            });
            if all_present {
                teams_result.compliant += 1;
            } else {
                teams_result.issues.push(repo.name.clone());
            }
        }
    }

    Ok(SystemPlan {
        id: sys_id.to_string(),
        repo_count: repos.len(),
        security: security_result,
        branch_protection: protection_result,
        rulesets: rulesets_result,
        teams: teams_result,
    })
}

fn count_actions(plan: &SystemPlan) -> usize {
    plan.security.issues.len()
        + plan.branch_protection.issues.len()
        + plan.rulesets.issues.len()
        + plan.teams.issues.len()
}

fn print_report(report: &PlanReport) {
    println!();
    println!("  {}", style("Ward Plan").bold().cyan());
    println!("  {}", style("=========").bold().cyan());

    for sys in &report.systems {
        println!();
        println!(
            "  System: {} ({} repositories)",
            style(&sys.id).bold(),
            sys.repo_count
        );

        print_category("Security", &sys.security);
        print_category("Branch Protection", &sys.branch_protection);
        print_category("Rulesets", &sys.rulesets);
        print_category("Teams", &sys.teams);
    }

    println!();
    println!(
        "  Summary: {} repos scanned, {} actions needed",
        style(report.total_repos).bold(),
        if report.total_actions > 0 {
            style(report.total_actions).red().bold()
        } else {
            style(report.total_actions).green().bold()
        }
    );
}

fn print_category(name: &str, result: &CategoryResult) {
    println!();
    println!("    {}", style(name).underlined());
    println!(
        "      {}/{} in compliance",
        style(result.compliant).green(),
        result.total
    );

    if !result.issues.is_empty() {
        println!(
            "      {} repos need changes: {}",
            result.issues.len(),
            result.issues.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_summary_counts() {
        let plan = SystemPlan {
            id: "backend".to_string(),
            repo_count: 5,
            security: CategoryResult {
                compliant: 3,
                total: 5,
                issues: vec!["repo-a".to_string(), "repo-b".to_string()],
            },
            branch_protection: CategoryResult {
                compliant: 5,
                total: 5,
                issues: vec![],
            },
            rulesets: CategoryResult {
                compliant: 4,
                total: 5,
                issues: vec!["repo-c".to_string()],
            },
            teams: CategoryResult {
                compliant: 5,
                total: 5,
                issues: vec![],
            },
        };

        assert_eq!(count_actions(&plan), 3);
        assert_eq!(plan.security.compliant, 3);
        assert_eq!(plan.branch_protection.compliant, 5);
    }

    #[test]
    fn test_plan_json_structure() {
        let report = PlanReport {
            systems: vec![SystemPlan {
                id: "backend".to_string(),
                repo_count: 3,
                security: CategoryResult {
                    compliant: 3,
                    total: 3,
                    issues: vec![],
                },
                branch_protection: CategoryResult {
                    compliant: 2,
                    total: 3,
                    issues: vec!["repo-x".to_string()],
                },
                rulesets: CategoryResult {
                    compliant: 3,
                    total: 3,
                    issues: vec![],
                },
                teams: CategoryResult {
                    compliant: 3,
                    total: 3,
                    issues: vec![],
                },
            }],
            total_repos: 3,
            total_actions: 1,
        };

        let json_str = serde_json::to_string_pretty(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["total_repos"], 3);
        assert_eq!(parsed["total_actions"], 1);
        assert_eq!(parsed["systems"][0]["id"], "backend");
        assert_eq!(parsed["systems"][0]["security"]["compliant"], 3);
        assert_eq!(
            parsed["systems"][0]["branch_protection"]["issues"][0],
            "repo-x"
        );
    }

    #[test]
    fn test_plan_all_systems() {
        let toml_str = r#"
            [org]
            name = "org"
            [[systems]]
            id = "backend"
            name = "Backend"
            [[systems]]
            id = "frontend"
            name = "Frontend"
        "#;
        let manifest: Manifest = toml::from_str(toml_str).unwrap();

        let ids = resolve_system_ids(&manifest, None, true).unwrap();
        assert_eq!(ids, vec!["backend", "frontend"]);

        let single = resolve_system_ids(&manifest, Some("backend"), false).unwrap();
        assert_eq!(single, vec!["backend"]);
    }

    #[test]
    fn test_plan_no_systems_errors() {
        let manifest = Manifest::default();
        let result = resolve_system_ids(&manifest, None, true);
        assert!(result.is_err());
    }
}
