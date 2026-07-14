use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;

use crate::cli::drift::{compare_protection, compare_security};
use crate::config::Manifest;
use crate::config::manifest::{BranchProtectionConfig, SecurityConfig, TeamAccess};
use crate::github::Client;
use crate::github::branch_protection::BranchProtectionState;
use crate::github::repos::Repository;
use crate::github::rulesets::Ruleset;
use crate::github::security::SecurityState;
use crate::github::teams::Team;
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
            self.run_legacy(client, manifest, system, repo, json).await
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
        repo: Option<&str>,
        json: bool,
    ) -> Result<()> {
        let repos = if let Some(repo_name) = repo {
            vec![client.get_repo(repo_name).await?]
        } else {
            Vec::new()
        };

        let system_ids = if !repos.is_empty() {
            vec![resolve_legacy_system_id(manifest, system, &repos[0])]
        } else {
            resolve_system_ids(manifest, system, self.all)?
        };

        let mut report = PlanReport {
            systems: Vec::new(),
            total_repos: 0,
            total_actions: 0,
        };

        for sys_id in &system_ids {
            let sys_repos = if !repos.is_empty() {
                repos.clone()
            } else {
                let excludes = manifest.exclude_patterns_for_system(sys_id);
                let explicit = manifest.explicit_repos_for_system(sys_id);
                client
                    .list_repos_for_system(
                        sys_id,
                        manifest.matches_prefix_for_system(sys_id),
                        &excludes,
                        &explicit,
                    )
                    .await?
            };

            if sys_repos.is_empty() {
                continue;
            }

            let sys_plan = check_system(client, manifest, sys_id, &sys_repos).await?;
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

fn resolve_legacy_system_id(
    manifest: &Manifest,
    system: Option<&str>,
    repo: &Repository,
) -> String {
    system
        .map(|s| s.to_string())
        .or_else(|| manifest.system_for_repo(&repo.name).map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Outcome of checking one repository against one category.
/// `Ok(true)` means compliant, `Ok(false)` means drifted, and `Err(())` means the
/// API read failed so the repo is treated as an issue for that category.
type CheckOutcome = Result<bool, ()>;

fn record_outcome(outcome: CheckOutcome, category: &mut CategoryResult, repo_name: &str) {
    match outcome {
        Ok(true) => category.compliant += 1,
        _ => category.issues.push(repo_name.to_string()),
    }
}

fn check_security_desired(
    desired: &SecurityConfig,
    state: Result<&SecurityState, ()>,
) -> CheckOutcome {
    Ok(compare_security(desired, state?).is_empty())
}

fn check_protection_desired(
    desired: &BranchProtectionConfig,
    state: Option<&BranchProtectionState>,
) -> bool {
    let default = BranchProtectionState::default();
    let state = state.unwrap_or(&default);
    compare_protection(desired, state).is_empty()
}

fn check_rulesets_desired(expected: &[&str], rulesets: Result<&[Ruleset], ()>) -> CheckOutcome {
    let rulesets = rulesets?;
    Ok(expected
        .iter()
        .all(|name| rulesets.iter().any(|ruleset| ruleset.name == *name)))
}

fn check_teams_desired(desired: &[TeamAccess], teams: Result<&[Team], ()>) -> CheckOutcome {
    let teams = teams?;
    Ok(desired.iter().all(|desired_team| {
        teams.iter().any(|team| {
            team.slug == desired_team.slug && team.permission == desired_team.permission
        })
    }))
}

async fn check_system(
    client: &Client,
    manifest: &Manifest,
    sys_id: &str,
    repos: &[Repository],
) -> Result<SystemPlan> {
    let desired_security = manifest.security_for_system(sys_id);
    let desired_protection = &manifest.branch_protection;

    let mut security_result = new_category_result(repos.len());
    let mut protection_result = new_category_result(repos.len());
    let mut rulesets_result = new_category_result(repos.len());
    let mut teams_result = new_category_result(repos.len());
    let mut repository_settings_result = new_category_result(repos.len());
    let mut files_result = new_category_result(repos.len());

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
        // Security: any read error is recorded as an issue.
        let sec_result = client.get_security_state(&repo.name).await;
        let sec_state = sec_result.as_ref().map_err(|_| ());
        record_outcome(
            check_security_desired(desired_security, sec_state),
            &mut security_result,
            &repo.name,
        );

        // Branch protection: missing protection is evaluated as default state.
        let prot_outcome = match client
            .get_branch_protection(&repo.name, &repo.default_branch)
            .await
        {
            Ok(Some(ref state)) => Ok(check_protection_desired(desired_protection, Some(state))),
            Ok(None) => Ok(check_protection_desired(desired_protection, None)),
            Err(_) => Err(()),
        };
        record_outcome(prot_outcome, &mut protection_result, &repo.name);

        // Repository settings are evaluated independently of branch protection reads.
        let settings_outcome = if let Some(desired) = &manifest.repository {
            crate::cli::settings::repository_settings_compliant(client, &repo.name, desired)
                .await
                .map_err(|_| ())
        } else {
            Ok(true)
        };
        record_outcome(
            settings_outcome,
            &mut repository_settings_result,
            &repo.name,
        );

        // Rulesets
        let rulesets_outcome = if expected_rulesets.is_empty() {
            Ok(true)
        } else {
            let rulesets_result = client.list_repository_rulesets(&repo.name).await;
            check_rulesets_desired(
                &expected_rulesets,
                rulesets_result.as_ref().map(Vec::as_slice).map_err(|_| ()),
            )
        };
        record_outcome(rulesets_outcome, &mut rulesets_result, &repo.name);

        // Teams
        let teams_outcome = if desired_teams.is_empty() {
            Ok(true)
        } else {
            let teams_result = client.list_repo_teams(&repo.name).await;
            check_teams_desired(
                desired_teams,
                teams_result.as_ref().map(Vec::as_slice).map_err(|_| ()),
            )
        };
        record_outcome(teams_outcome, &mut teams_result, &repo.name);

        // Managed files
        let files_outcome = if manifest.files.is_empty() {
            Ok(true)
        } else {
            crate::cli::commit::managed_files_compliant(client, &repo.name, manifest)
                .await
                .map_err(|_| ())
        };
        record_outcome(files_outcome, &mut files_result, &repo.name);
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

fn new_category_result(total: usize) -> CategoryResult {
    CategoryResult {
        compliant: 0,
        total,
        issues: Vec::new(),
    }
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

    #[test]
    fn test_record_outcome_keeps_counts_consistent() {
        let mut category = new_category_result(3);
        record_outcome(Ok(true), &mut category, "repo-a");
        record_outcome(Ok(false), &mut category, "repo-b");
        record_outcome(Err(()), &mut category, "repo-c");

        assert_eq!(category.compliant, 1);
        assert_eq!(category.issues, vec!["repo-b", "repo-c"]);
        assert_eq!(category.compliant + category.issues.len(), category.total);
    }

    #[test]
    fn test_check_security_desired_detects_drift_and_read_error() {
        let desired = SecurityConfig {
            secret_scanning: true,
            ..SecurityConfig::default()
        };
        let compliant = SecurityState {
            secret_scanning: true,
            ..SecurityState::default()
        };
        let drifted = SecurityState {
            secret_scanning: false,
            ..SecurityState::default()
        };

        assert!(check_security_desired(&desired, Ok(&compliant)).unwrap());
        assert!(!check_security_desired(&desired, Ok(&drifted)).unwrap());
        assert!(check_security_desired(&desired, Err(())).is_err());
    }

    #[test]
    fn test_check_protection_desired_treats_missing_as_default() {
        let desired = BranchProtectionConfig::default();
        assert!(check_protection_desired(&desired, None));

        let state = BranchProtectionState {
            required_pull_request_reviews: true,
            ..BranchProtectionState::default()
        };
        assert!(!check_protection_desired(&desired, Some(&state)));
    }

    #[test]
    fn test_check_rulesets_desired_detects_missing_and_read_error() {
        let rulesets = vec![Ruleset {
            id: 1,
            name: "Branch Protection".to_string(),
            target: String::new(),
            source_type: String::new(),
            source: String::new(),
            enforcement: String::new(),
            conditions: None,
            rules: Vec::new(),
            bypass_actors: Vec::new(),
        }];

        assert!(check_rulesets_desired(&["Branch Protection"], Ok(&rulesets)).unwrap());
        assert!(!check_rulesets_desired(&["Missing Ruleset"], Ok(&rulesets)).unwrap());
        assert!(check_rulesets_desired(&["Branch Protection"], Err(())).is_err());
    }

    #[test]
    fn test_check_teams_desired_detects_missing_and_read_error() {
        let desired = vec![TeamAccess {
            slug: "admins".to_string(),
            permission: "admin".to_string(),
        }];
        let teams = vec![Team {
            id: 1,
            name: "Admins".to_string(),
            slug: "admins".to_string(),
            description: None,
            permission: "admin".to_string(),
            privacy: "closed".to_string(),
        }];

        assert!(check_teams_desired(&desired, Ok(&teams)).unwrap());
        assert!(!check_teams_desired(&desired, Ok(&[])).unwrap());
        assert!(check_teams_desired(&desired, Err(())).is_err());
    }

    #[test]
    fn test_settings_check_is_independent_of_protection_failure() {
        let mut protection = new_category_result(1);
        let mut settings = new_category_result(1);

        record_outcome(Err(()), &mut protection, "repo-a");
        record_outcome(Ok(true), &mut settings, "repo-a");

        assert_eq!(protection.issues, vec!["repo-a"]);
        assert_eq!(settings.compliant, 1);
        assert!(settings.issues.is_empty());
    }

    #[test]
    fn test_resolve_legacy_system_id_prefers_system_then_repo_match() {
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
        let backend_repo = Repository {
            name: "backend-api".to_string(),
            full_name: "org/backend-api".to_string(),
            archived: false,
            default_branch: "main".to_string(),
            description: None,
            visibility: "internal".to_string(),
            language: None,
            security_and_analysis: None,
            topics: Vec::new(),
        };
        let unknown_repo = Repository {
            name: "random".to_string(),
            full_name: "org/random".to_string(),
            archived: false,
            default_branch: "main".to_string(),
            description: None,
            visibility: "internal".to_string(),
            language: None,
            security_and_analysis: None,
            topics: Vec::new(),
        };

        assert_eq!(
            resolve_legacy_system_id(&manifest, Some("frontend"), &backend_repo),
            "frontend"
        );
        assert_eq!(
            resolve_legacy_system_id(&manifest, None, &backend_repo),
            "backend"
        );
        assert_eq!(resolve_legacy_system_id(&manifest, None, &unknown_repo), "");
    }

    #[test]
    fn test_legacy_repo_filtering_uses_single_repo_and_derived_system() {
        let toml_str = r#"
            [org]
            name = "org"
            [[systems]]
            id = "backend"
            name = "Backend"
            repos = ["backend-api"]
        "#;
        let manifest: Manifest = toml::from_str(toml_str).unwrap();
        let repo = Repository {
            name: "backend-api".to_string(),
            full_name: "org/backend-api".to_string(),
            archived: false,
            default_branch: "main".to_string(),
            description: None,
            visibility: "internal".to_string(),
            language: None,
            security_and_analysis: None,
            topics: Vec::new(),
        };

        assert_eq!(resolve_legacy_system_id(&manifest, None, &repo), "backend");
    }
}
