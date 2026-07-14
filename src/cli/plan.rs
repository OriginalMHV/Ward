use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;

use crate::cli::drift::{compare_protection, compare_security};
use crate::config::Manifest;
use crate::config::manifest::TeamAccess;
use crate::github::Client;
use crate::github::repos::Repository;
use crate::reconcile::unified::{self, UnifiedOptions};

#[derive(Args)]
pub struct PlanCommand {
    /// Compatibility flag; v2 already checks all configured systems by default
    #[arg(long)]
    all: bool,

    /// Limit to one or more categories (repeatable). Valid: repository, files,
    /// security, rulesets, branch-protection, actions, environments, access,
    /// integrations.
    #[arg(long = "category", value_name = "CATEGORY")]
    categories: Vec<String>,

    /// Allow planning high-impact repository changes (visibility, archive)
    #[arg(long)]
    allow_high_impact: bool,
}

impl PlanCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
        json: bool,
    ) -> Result<()> {
        if is_v2_manifest(manifest) {
            self.run_v2(client, manifest, system, repo, json).await
        } else {
            self.run_legacy(client, manifest, system, json).await
        }
    }

    async fn run_v2(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
        json: bool,
    ) -> Result<()> {
        let categories = unified::parse_categories(&self.categories)?;
        let options = UnifiedOptions {
            categories,
            allow_high_impact: self.allow_high_impact,
        };

        let repos = unified::resolve_target_repos(client, manifest, system, repo).await?;
        if repos.is_empty() {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&unified::UnifiedReport::from_repos(Vec::new()))
                        .unwrap_or_default()
                );
            } else {
                println!("  No matching repositories found.");
            }
            return Ok(());
        }

        let report = unified::plan(client, manifest, &repos, &options).await?;

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        } else {
            unified::render_report(&report, "Ward Plan");
        }

        Ok(())
    }

    async fn run_legacy(
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
                .list_repos_for_system(
                    sys_id,
                    manifest.matches_prefix_for_system(sys_id),
                    &excludes,
                    &explicit,
                )
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

fn is_v2_manifest(manifest: &Manifest) -> bool {
    manifest.v2_schema().is_some() || !manifest.v2_categories().is_empty()
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
    repository_settings: CategoryResult,
    branch_protection: CategoryResult,
    rulesets: CategoryResult,
    teams: CategoryResult,
    files: CategoryResult,
}

#[derive(Debug, Serialize)]
struct CategoryResult {
    compliant: usize,
    total: usize,
    issues: Vec<String>,
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
    let mut repository_settings_result = CategoryResult {
        compliant: 0,
        total: repos.len(),
        issues: Vec::new(),
    };
    let mut files_result = CategoryResult {
        compliant: 0,
        total: repos.len(),
        issues: Vec::new(),
    };

    let desired_teams: &[TeamAccess] = manifest
        .system(sys_id)
        .map(|s| s.teams.as_slice())
        .unwrap_or(&[]);

    let expected_rulesets: Vec<&str> = if manifest.rulesets.repository.is_empty() {
        manifest
            .rulesets
            .branch_protection
            .as_ref()
            .filter(|config| config.enabled)
            .map(|config| vec![config.name.as_deref().unwrap_or("Branch Protection")])
            .unwrap_or_default()
    } else {
        manifest
            .rulesets
            .repository
            .iter()
            .map(|ruleset| ruleset.name.as_str())
            .collect()
    };

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

            // Repository settings
            if let Some(desired) = &manifest.repository {
                match crate::cli::settings::repository_settings_compliant(
                    client, &repo.name, desired,
                )
                .await
                {
                    Ok(true) => repository_settings_result.compliant += 1,
                    _ => repository_settings_result.issues.push(repo.name.clone()),
                }
            } else {
                repository_settings_result.compliant += 1;
            }
        }

        // Rulesets
        if expected_rulesets.is_empty() {
            rulesets_result.compliant += 1;
        } else if let Ok(rulesets) = client.list_repository_rulesets(&repo.name).await {
            if expected_rulesets.iter().all(|expected_name| {
                rulesets
                    .iter()
                    .any(|ruleset| ruleset.name == *expected_name)
            }) {
                rulesets_result.compliant += 1;
            } else {
                rulesets_result.issues.push(repo.name.clone());
            }
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

        // Managed files
        if manifest.files.is_empty() {
            files_result.compliant += 1;
        } else {
            match crate::cli::commit::managed_files_compliant(client, &repo.name, manifest).await {
                Ok(true) => files_result.compliant += 1,
                _ => files_result.issues.push(repo.name.clone()),
            }
        }
    }

    Ok(SystemPlan {
        id: sys_id.to_string(),
        repo_count: repos.len(),
        security: security_result,
        repository_settings: repository_settings_result,
        branch_protection: protection_result,
        rulesets: rulesets_result,
        teams: teams_result,
        files: files_result,
    })
}

fn count_actions(plan: &SystemPlan) -> usize {
    plan.security.issues.len()
        + plan.repository_settings.issues.len()
        + plan.branch_protection.issues.len()
        + plan.rulesets.issues.len()
        + plan.teams.issues.len()
        + plan.files.issues.len()
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
        print_category("Repository Settings", &sys.repository_settings);
        print_category("Branch Protection", &sys.branch_protection);
        print_category("Rulesets", &sys.rulesets);
        print_category("Teams", &sys.teams);
        print_category("Managed Files", &sys.files);
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
            repository_settings: CategoryResult {
                compliant: 5,
                total: 5,
                issues: vec![],
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
            files: CategoryResult {
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
                repository_settings: CategoryResult {
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
                files: CategoryResult {
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

    #[test]
    fn test_is_v2_manifest_detection() {
        let legacy = Manifest::default();
        assert!(!is_v2_manifest(&legacy));

        let mut v2 = Manifest::default();
        v2.v2.schema = Some(crate::config::manifest::ManifestSchema::v2());
        assert!(is_v2_manifest(&v2));
    }
}
