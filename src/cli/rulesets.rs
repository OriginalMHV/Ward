use std::collections::HashMap;

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::config::manifest::{
    BypassTeam, RepositoryRulesetConfig, RulesetBranchProtection, RulesetBypassActorConfig,
};
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
            RulesetsAction::Apply { yes } => {
                crate::reconcile::unified::guard_legacy_mutation(
                    manifest,
                    crate::reconcile::unified::Category::Rulesets,
                    "rulesets apply",
                )?;
                apply(client, manifest, system, repo, *yes).await
            }
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
        .list_repos_for_system(
            sys,
            manifest.matches_prefix_for_system(sys),
            &excludes,
            &explicit,
        )
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
    body: serde_json::Value,
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
) -> Result<Vec<RulesetPlan>> {
    let repos = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Scanning {} repositories for rulesets...",
        style("[..]").dim(),
        repos.len()
    );

    let mut plans = Vec::new();
    let mut team_id_cache: HashMap<String, u64> = HashMap::new();

    for repo_name in &repos {
        let rulesets = client.list_repository_rulesets(repo_name).await?;

        if !manifest.rulesets.repository.is_empty() {
            for config in &manifest.rulesets.repository {
                let body =
                    build_repository_ruleset_json(client, config, &mut team_id_cache).await?;
                plans.push(build_ruleset_plan(client, repo_name, &rulesets, body).await);
            }
        } else {
            let config = match system {
                Some(sys) => manifest.rulesets_branch_protection_for_system(sys),
                None => manifest.rulesets.branch_protection.clone(),
            };
            let config = match config {
                Some(config) if config.enabled => config,
                _ => anyhow::bail!(
                    "No repository rulesets or enabled rulesets.branch_protection configured in ward.toml"
                ),
            };

            let repo_config = config.for_repo(repo_name);
            let bypass_actors =
                resolve_bypass_actors(client, &repo_config.bypass_teams, &mut team_id_cache)
                    .await?;
            let body = build_ruleset_json(&repo_config, &bypass_actors);
            plans.push(build_ruleset_plan(client, repo_name, &rulesets, body).await);
        }
    }

    Ok(plans)
}

async fn build_ruleset_plan(
    client: &Client,
    repo: &str,
    existing_rulesets: &[crate::github::rulesets::Ruleset],
    body: serde_json::Value,
) -> RulesetPlan {
    let expected_name = body["name"].as_str().unwrap_or("Ruleset");
    let existing = existing_rulesets
        .iter()
        .find(|ruleset| ruleset.name == expected_name);

    let action = match existing {
        None => RulesetPlanAction::Create {
            name: expected_name.to_owned(),
        },
        Some(ruleset) => {
            let detail = client.get_ruleset(repo, ruleset.id).await;
            match detail {
                Ok(detail) if ruleset_matches(&detail, &body) => RulesetPlanAction::InSync {
                    name: expected_name.to_owned(),
                },
                _ => RulesetPlanAction::Update {
                    id: ruleset.id,
                    name: expected_name.to_owned(),
                },
            }
        }
    };

    RulesetPlan {
        repo: repo.to_owned(),
        body,
        action,
    }
}

async fn build_repository_ruleset_json(
    client: &Client,
    config: &RepositoryRulesetConfig,
    team_id_cache: &mut HashMap<String, u64>,
) -> Result<serde_json::Value> {
    let conditions = config
        .conditions_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .with_context(|| format!("Invalid conditions_json for ruleset {}", config.name))?
        .unwrap_or(serde_json::Value::Null);

    let rules = config
        .rules
        .iter()
        .map(|rule| {
            let mut body = serde_json::json!({ "type": rule.rule_type });
            if let Some(parameters) = rule.parameters_json.as_deref() {
                body["parameters"] = serde_json::from_str(parameters).with_context(|| {
                    format!(
                        "Invalid parameters_json for {} rule in ruleset {}",
                        rule.rule_type, config.name
                    )
                })?;
            }
            Ok(body)
        })
        .collect::<Result<Vec<_>>>()?;

    let bypass_actors =
        resolve_repository_bypass_actors(client, &config.bypass_actors, team_id_cache).await?;

    Ok(serde_json::json!({
        "name": config.name,
        "target": config.target,
        "enforcement": config.enforcement,
        "conditions": conditions,
        "rules": rules,
        "bypass_actors": bypass_actors,
    }))
}

async fn resolve_repository_bypass_actors(
    client: &Client,
    actors: &[RulesetBypassActorConfig],
    cache: &mut HashMap<String, u64>,
) -> Result<Vec<serde_json::Value>> {
    let mut resolved = Vec::with_capacity(actors.len());

    for actor in actors {
        let actor_id = if actor.actor_type == "Team" {
            if let Some(slug) = actor.team_slug.as_deref() {
                match cache.get(slug) {
                    Some(id) => Some(*id),
                    None => {
                        let id = client.get_team_id(slug).await?;
                        cache.insert(slug.to_owned(), id);
                        Some(id)
                    }
                }
            } else {
                actor.actor_id
            }
        } else {
            actor.actor_id
        };

        resolved.push(serde_json::json!({
            "actor_id": actor_id,
            "actor_type": actor.actor_type,
            "bypass_mode": actor.bypass_mode,
        }));
    }

    Ok(resolved)
}

async fn plan(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let plans = build_plans(client, manifest, system, repo).await?;

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
    let plans = build_plans(client, manifest, system, repo).await?;

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

    let mut succeeded = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    for plan in &plans {
        match &plan.action {
            RulesetPlanAction::InSync { .. } => {}
            RulesetPlanAction::Create { name } => {
                match client.create_ruleset(&plan.repo, &plan.body).await {
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
                match client.update_ruleset(&plan.repo, *id, &plan.body).await {
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

/// Compare a deployed ruleset against the reproducible GitHub API fields.
fn ruleset_matches(deployed: &RulesetDetail, expected: &serde_json::Value) -> bool {
    if deployed.name != expected["name"].as_str().unwrap_or_default()
        || deployed.target != expected["target"].as_str().unwrap_or("branch")
    {
        return false;
    }

    if deployed.enforcement != expected["enforcement"].as_str().unwrap_or("active") {
        return false;
    }

    let expected_conditions = expected
        .get("conditions")
        .unwrap_or(&serde_json::Value::Null);
    let deployed_conditions = deployed
        .conditions
        .as_ref()
        .unwrap_or(&serde_json::Value::Null);
    if deployed_conditions != expected_conditions {
        return false;
    }

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
        let expected_params = expected_rule
            .get("parameters")
            .filter(|parameters| !parameters.is_null());
        match (deployed_rule.parameters.as_ref(), expected_params) {
            (Some(deployed_params), Some(expected_params))
                if deployed_params == expected_params => {}
            (None, None) => {}
            (Some(deployed_params), None) if deployed_params.is_null() => {}
            _ => return false,
        }
    }

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

    #[tokio::test]
    async fn imported_ruleset_preserves_arbitrary_rules() {
        let config = RepositoryRulesetConfig {
            name: "Release tags".to_owned(),
            target: "tag".to_owned(),
            enforcement: "active".to_owned(),
            conditions_json: Some(
                r#"{"ref_name":{"include":["refs/tags/v*"],"exclude":[]}}"#.to_owned(),
            ),
            rules: vec![crate::config::manifest::RepositoryRuleConfig {
                rule_type: "required_signatures".to_owned(),
                parameters_json: None,
            }],
            bypass_actors: Vec::new(),
        };
        let client = Client::new_for_test("acme", "http://unused");

        let body = build_repository_ruleset_json(&client, &config, &mut HashMap::new())
            .await
            .unwrap();

        assert_eq!(body["name"], "Release tags");
        assert_eq!(body["target"], "tag");
        assert_eq!(body["rules"][0]["type"], "required_signatures");
        assert_eq!(body["conditions"]["ref_name"]["include"][0], "refs/tags/v*");
    }

    #[test]
    fn ruleset_match_checks_conditions_and_target() {
        let deployed = RulesetDetail {
            id: 1,
            name: "Release tags".to_owned(),
            enforcement: "active".to_owned(),
            target: "tag".to_owned(),
            rules: vec![crate::github::rulesets::RulesetRule {
                rule_type: "required_signatures".to_owned(),
                parameters: None,
            }],
            conditions: Some(serde_json::json!({
                "ref_name": {
                    "include": ["refs/tags/v*"],
                    "exclude": []
                }
            })),
            bypass_actors: Vec::new(),
        };
        let expected = serde_json::json!({
            "name": "Release tags",
            "target": "tag",
            "enforcement": "active",
            "conditions": {
                "ref_name": {
                    "include": ["refs/tags/v*"],
                    "exclude": []
                }
            },
            "rules": [{ "type": "required_signatures" }],
            "bypass_actors": []
        });

        assert!(ruleset_matches(&deployed, &expected));

        let mut wrong_target = expected;
        wrong_target["target"] = serde_json::json!("branch");
        assert!(!ruleset_matches(&deployed, &wrong_target));
    }
}
