use std::collections::HashMap;

use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::config::manifest::{BypassTeam, RulesetBranchProtection};
use crate::github::Client;
use crate::github::rulesets::RulesetDetail;

#[derive(Args)]
pub struct RulesetsCommand {
    #[command(subcommand)]
    action: RulesetsAction,
}

#[derive(clap::Subcommand)]
enum RulesetsAction {
    /// Preview ruleset changes (dry-run)
    Plan,

    /// Apply rulesets to repositories
    Apply {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },

    /// Show current rulesets across repos
    Audit,
}

impl RulesetsCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        match &self.action {
            RulesetsAction::Plan => plan(client, manifest, system, repo).await,
            RulesetsAction::Apply { yes } => apply(client, manifest, system, repo, *yes).await,
            RulesetsAction::Audit => audit(client, manifest, system, repo).await,
        }
    }
}

async fn resolve_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<String>> {
    if let Some(repo_name) = repo {
        return Ok(vec![repo_name.to_owned()]);
    }

    let sys = system.ok_or_else(|| {
        anyhow::anyhow!("Either --system or --repo is required for rulesets commands")
    })?;

    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(sys, &excludes, &explicit)
        .await?;
    Ok(repos.into_iter().map(|r| r.name).collect())
}

/// Build the GitHub API JSON body for a branch protection ruleset.
pub fn build_ruleset_json(
    config: &RulesetBranchProtection,
    bypass_actors: &[serde_json::Value],
) -> serde_json::Value {
    let name = config.name.as_deref().unwrap_or("Branch Protection");

    let mut rules = vec![serde_json::json!({
        "type": "pull_request",
        "parameters": {
            "required_approving_review_count": config.required_approvals,
            "dismiss_stale_reviews_on_push": config.dismiss_stale_reviews,
            "require_code_owner_review": config.require_code_owner_reviews,
            "require_last_push_approval": false,
            "required_review_thread_resolution": false
        }
    })];

    if config.block_force_pushes {
        rules.push(serde_json::json!({"type": "non_fast_forward"}));
    }

    if config.block_deletions {
        rules.push(serde_json::json!({"type": "deletion"}));
    }

    if config.require_linear_history {
        rules.push(serde_json::json!({"type": "required_linear_history"}));
    }

    if !config.required_status_checks.is_empty() {
        let checks: Vec<serde_json::Value> = config
            .required_status_checks
            .iter()
            .map(|c| serde_json::json!({"context": c}))
            .collect();
        rules.push(serde_json::json!({
            "type": "required_status_checks",
            "parameters": {
                "required_status_checks": checks,
                "strict_required_status_checks_policy": false
            }
        }));
    }

    serde_json::json!({
        "name": name,
        "target": "branch",
        "enforcement": config.enforcement,
        "conditions": {
            "ref_name": {
                "include": ["~DEFAULT_BRANCH"],
                "exclude": []
            }
        },
        "rules": rules,
        "bypass_actors": bypass_actors
    })
}

struct RulesetPlan {
    repo: String,
    action: RulesetPlanAction,
}

enum RulesetPlanAction {
    Create { name: String },
    Update { id: u64, name: String },
    InSync { name: String },
}

async fn build_plans(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<(Vec<RulesetPlan>, RulesetBranchProtection)> {
    let config = match system {
        Some(sys) => manifest.rulesets_branch_protection_for_system(sys),
        None => manifest.rulesets.branch_protection.clone(),
    };

    let config = match config {
        Some(c) if c.enabled => c,
        _ => {
            anyhow::bail!("No rulesets.branch_protection configured or not enabled in ward.toml");
        }
    };

    let repos = resolve_repos(client, manifest, system, repo).await?;
    let expected_name = config.name.as_deref().unwrap_or("Branch Protection");

    println!();
    println!(
        "  {} Scanning {} repositories for rulesets...",
        style("[..]").dim(),
        repos.len()
    );

    let mut plans = Vec::new();
    let mut team_id_cache: HashMap<String, u64> = HashMap::new();

    for repo_name in &repos {
        let rulesets = client.list_rulesets(repo_name).await?;
        let existing = rulesets.iter().find(|r| r.name == expected_name);

        let action = match existing {
            None => RulesetPlanAction::Create {
                name: expected_name.to_string(),
            },
            Some(r) => {
                let repo_config = config.for_repo(repo_name);
                let bypass_actors =
                    resolve_bypass_actors(client, &repo_config.bypass_teams, &mut team_id_cache)
                        .await?;
                let expected_body = build_ruleset_json(&repo_config, &bypass_actors);

                let detail = client.get_ruleset(repo_name, r.id).await;
                match detail {
                    Ok(d) if ruleset_matches(&d, &expected_body) => RulesetPlanAction::InSync {
                        name: expected_name.to_string(),
                    },
                    _ => RulesetPlanAction::Update {
                        id: r.id,
                        name: expected_name.to_string(),
                    },
                }
            }
        };

        plans.push(RulesetPlan {
            repo: repo_name.clone(),
            action,
        });
    }

    Ok((plans, config))
}

async fn plan(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let (plans, _config) = build_plans(client, manifest, system, repo).await?;

    print_plan_table(&plans);

    let needs_changes = plans
        .iter()
        .filter(|p| !matches!(p.action, RulesetPlanAction::InSync { .. }))
        .count();
    if needs_changes > 0 {
        println!(
            "\n  Run {} to apply these changes.",
            style("ward rulesets apply").cyan().bold()
        );
    }

    Ok(())
}

async fn apply(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    yes: bool,
) -> Result<()> {
    let (plans, config) = build_plans(client, manifest, system, repo).await?;

    let needs_changes: Vec<&RulesetPlan> = plans
        .iter()
        .filter(|p| !matches!(p.action, RulesetPlanAction::InSync { .. }))
        .collect();

    if needs_changes.is_empty() {
        println!(
            "\n  {} All repositories have rulesets up to date.",
            style("[ok]").green()
        );
        return Ok(());
    }

    print_plan_table(&plans);

    if !yes {
        let proceed = Confirm::new()
            .with_prompt(format!(
                "  Apply rulesets to {} repositories?",
                needs_changes.len()
            ))
            .default(false)
            .interact()?;

        if !proceed {
            println!("  Aborted.");
            return Ok(());
        }
    }

    println!();
    println!("  {} Applying rulesets...", style("[..]").dim());

    let mut team_id_cache: HashMap<String, u64> = HashMap::new();
    let mut succeeded = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    for plan in &plans {
        match &plan.action {
            RulesetPlanAction::InSync { .. } => {}
            RulesetPlanAction::Create { name } => {
                let repo_config = config.for_repo(&plan.repo);
                let bypass_actors =
                    resolve_bypass_actors(client, &repo_config.bypass_teams, &mut team_id_cache)
                        .await?;
                let body = build_ruleset_json(&repo_config, &bypass_actors);
                match client.create_ruleset(&plan.repo, &body).await {
                    Ok(_) => {
                        println!(
                            "  {} {}: created {}",
                            style("[ok]").green(),
                            plan.repo,
                            name
                        );
                        succeeded += 1;
                    }
                    Err(e) => {
                        println!("  {} {}: {}", style("[!!]").red(), plan.repo, e);
                        failed.push((plan.repo.clone(), e.to_string()));
                    }
                }
            }
            RulesetPlanAction::Update { id, name } => {
                let repo_config = config.for_repo(&plan.repo);
                let bypass_actors =
                    resolve_bypass_actors(client, &repo_config.bypass_teams, &mut team_id_cache)
                        .await?;
                let body = build_ruleset_json(&repo_config, &bypass_actors);
                match client.update_ruleset(&plan.repo, *id, &body).await {
                    Ok(()) => {
                        println!(
                            "  {} {}: updated {}",
                            style("[ok]").green(),
                            plan.repo,
                            name
                        );
                        succeeded += 1;
                    }
                    Err(e) => {
                        println!("  {} {}: {}", style("[!!]").red(), plan.repo, e);
                        failed.push((plan.repo.clone(), e.to_string()));
                    }
                }
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!(
            "  {} All {} repositories updated successfully.",
            style("[ok]").green(),
            succeeded
        );
    } else {
        println!(
            "  {} {} succeeded, {} failed:",
            style("[!!]").yellow(),
            succeeded,
            failed.len()
        );
        for (repo, err) in &failed {
            println!("    {} {}: {}", style("[!!]").red(), repo, err);
        }
    }

    Ok(())
}

async fn audit(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Auditing rulesets for {} repositories...",
        style("[..]").dim(),
        repos.len()
    );

    println!();
    println!(
        "  {} {}",
        style(format!("{:<40}", "Repository")).bold().underlined(),
        style("Rulesets").bold().underlined(),
    );
    println!("  {}", style("\u{2500}".repeat(70)).dim());

    for repo_name in &repos {
        let rulesets = client.list_rulesets(repo_name).await?;

        let summary = if rulesets.is_empty() {
            style("(none)").dim().to_string()
        } else {
            rulesets
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };

        println!("  {:<40} {}", repo_name, summary);
    }

    println!();
    println!(
        "  Summary: {} repositories scanned",
        style(repos.len()).green().bold()
    );

    Ok(())
}

/// Compare a deployed ruleset against the expected JSON body.
/// Checks enforcement, rule types + parameters, and bypass actors.
fn ruleset_matches(deployed: &RulesetDetail, expected: &serde_json::Value) -> bool {
    if deployed.enforcement != expected["enforcement"].as_str().unwrap_or("active") {
        return false;
    }

    // Compare rules (types + parameters)
    let expected_rules = match expected["rules"].as_array() {
        Some(r) => r,
        None => return false,
    };
    if deployed.rules.len() != expected_rules.len() {
        return false;
    }

    for expected_rule in expected_rules {
        let rule_type = match expected_rule["type"].as_str() {
            Some(t) => t,
            None => return false,
        };
        let deployed_rule = match deployed.rules.iter().find(|r| r.rule_type == rule_type) {
            Some(r) => r,
            None => return false,
        };
        // Compare parameters if present
        if let Some(expected_params) = expected_rule.get("parameters") {
            match &deployed_rule.parameters {
                Some(deployed_params) if deployed_params == expected_params => {}
                _ => return false,
            }
        }
    }

    // Compare bypass actors
    let expected_actors = match expected["bypass_actors"].as_array() {
        Some(a) => a,
        None => return deployed.bypass_actors.is_empty(),
    };
    if deployed.bypass_actors.len() != expected_actors.len() {
        return false;
    }

    for expected_actor in expected_actors {
        let found = deployed.bypass_actors.iter().any(|da| {
            da["actor_id"] == expected_actor["actor_id"]
                && da["actor_type"] == expected_actor["actor_type"]
                && da["bypass_mode"] == expected_actor["bypass_mode"]
        });
        if !found {
            return false;
        }
    }

    true
}

/// Resolve bypass teams to GitHub API bypass actor objects.
/// Uses a cache to avoid redundant API calls for the same team slug.
async fn resolve_bypass_actors(
    client: &Client,
    bypass_teams: &[BypassTeam],
    cache: &mut HashMap<String, u64>,
) -> Result<Vec<serde_json::Value>> {
    let mut actors = Vec::new();
    for team in bypass_teams {
        let slug = team.slug();
        let id = match cache.get(slug) {
            Some(&id) => id,
            None => {
                let id = client.get_team_id(slug).await?;
                cache.insert(slug.to_owned(), id);
                id
            }
        };
        actors.push(serde_json::json!({
            "actor_id": id,
            "actor_type": "Team",
            "bypass_mode": team.bypass_mode()
        }));
    }
    Ok(actors)
}

fn print_plan_table(plans: &[RulesetPlan]) {
    println!();
    println!("  {}", style("Rulesets Plan").bold().cyan());
    println!("  {}", style("\u{2500}".repeat(60)).dim());

    for plan in plans {
        match &plan.action {
            RulesetPlanAction::Create { name } => {
                println!(
                    "  {} {} -- create: {}",
                    style("[!!]").yellow(),
                    style(&plan.repo).bold(),
                    name
                );
            }
            RulesetPlanAction::Update { name, .. } => {
                println!(
                    "  {} {} -- update: {}",
                    style("[!!]").yellow(),
                    style(&plan.repo).bold(),
                    name
                );
            }
            RulesetPlanAction::InSync { name } => {
                println!(
                    "  {} {} -- {} (in sync)",
                    style("[ok]").green(),
                    style(&plan.repo).dim(),
                    name
                );
            }
        }
    }

    let needs_changes = plans
        .iter()
        .filter(|p| !matches!(p.action, RulesetPlanAction::InSync { .. }))
        .count();
    let up_to_date = plans.len() - needs_changes;

    println!();
    println!(
        "  Summary: {} need changes, {} up to date",
        if needs_changes > 0 {
            style(needs_changes).yellow().bold()
        } else {
            style(needs_changes).green().bold()
        },
        style(up_to_date).green()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_ruleset_json() {
        let config = RulesetBranchProtection {
            enabled: true,
            name: None,
            enforcement: "active".to_string(),
            required_approvals: 1,
            dismiss_stale_reviews: true,
            require_code_owner_reviews: false,
            required_status_checks: vec!["ci".to_string()],
            require_linear_history: false,
            block_force_pushes: true,
            block_deletions: true,
            bypass_teams: vec![],
            overrides: vec![],
        };

        let json = build_ruleset_json(&config, &[]);
        assert_eq!(json["name"], "Branch Protection");
        assert_eq!(json["target"], "branch");
        assert_eq!(json["enforcement"], "active");
        assert_eq!(
            json["conditions"]["ref_name"]["include"][0],
            "~DEFAULT_BRANCH"
        );

        let rules = json["rules"].as_array().unwrap();
        assert_eq!(rules[0]["type"], "pull_request");
        assert_eq!(rules[0]["parameters"]["required_approving_review_count"], 1);
        assert_eq!(
            rules[0]["parameters"]["dismiss_stale_reviews_on_push"],
            true
        );

        let rule_types: Vec<&str> = rules.iter().map(|r| r["type"].as_str().unwrap()).collect();
        assert!(rule_types.contains(&"non_fast_forward"));
        assert!(rule_types.contains(&"deletion"));
        assert!(rule_types.contains(&"required_status_checks"));
    }

    #[test]
    fn test_build_ruleset_json_minimal() {
        let config = RulesetBranchProtection {
            enabled: true,
            name: Some("Custom".to_string()),
            enforcement: "evaluate".to_string(),
            required_approvals: 2,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            required_status_checks: vec![],
            require_linear_history: false,
            block_force_pushes: false,
            block_deletions: false,
            bypass_teams: vec![],
            overrides: vec![],
        };

        let json = build_ruleset_json(&config, &[]);
        assert_eq!(json["name"], "Custom");
        assert_eq!(json["enforcement"], "evaluate");

        let rules = json["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["type"], "pull_request");
        assert_eq!(rules[0]["parameters"]["required_approving_review_count"], 2);
    }

    #[test]
    fn test_build_ruleset_json_with_linear_history() {
        let config = RulesetBranchProtection {
            enabled: true,
            name: None,
            enforcement: "active".to_string(),
            required_approvals: 1,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: true,
            required_status_checks: vec![],
            require_linear_history: true,
            block_force_pushes: false,
            block_deletions: false,
            bypass_teams: vec![],
            overrides: vec![],
        };

        let json = build_ruleset_json(&config, &[]);
        let rules = json["rules"].as_array().unwrap();
        let rule_types: Vec<&str> = rules.iter().map(|r| r["type"].as_str().unwrap()).collect();
        assert!(rule_types.contains(&"required_linear_history"));
        assert_eq!(rules[0]["parameters"]["require_code_owner_review"], true);
    }

    #[test]
    fn test_build_ruleset_json_with_bypass_actors() {
        let config = RulesetBranchProtection {
            enabled: true,
            name: None,
            enforcement: "active".to_string(),
            required_approvals: 1,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            required_status_checks: vec![],
            require_linear_history: false,
            block_force_pushes: false,
            block_deletions: false,
            bypass_teams: vec![BypassTeam::Simple("team-owners".to_string())],
            overrides: vec![],
        };

        let bypass_actors = vec![serde_json::json!({
            "actor_id": 12345,
            "actor_type": "Team",
            "bypass_mode": "always"
        })];

        let json = build_ruleset_json(&config, &bypass_actors);
        let actors = json["bypass_actors"].as_array().unwrap();
        assert_eq!(actors.len(), 1);
        assert_eq!(actors[0]["actor_id"], 12345);
        assert_eq!(actors[0]["actor_type"], "Team");
        assert_eq!(actors[0]["bypass_mode"], "always");
    }

    #[test]
    fn test_build_ruleset_json_empty_bypass_actors() {
        let config = RulesetBranchProtection {
            enabled: true,
            name: None,
            enforcement: "active".to_string(),
            required_approvals: 1,
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            required_status_checks: vec![],
            require_linear_history: false,
            block_force_pushes: false,
            block_deletions: false,
            bypass_teams: vec![],
            overrides: vec![],
        };

        let json = build_ruleset_json(&config, &[]);
        let actors = json["bypass_actors"].as_array().unwrap();
        assert!(actors.is_empty());
    }
}
