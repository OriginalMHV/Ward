use anyhow::{Context, Result};

use crate::reconcile::common::issue::{IssueSeverity, ReconcileIssue};
use crate::reconcile::common::issue::{has_blocker, write_outcome_issue};
use crate::reconcile::common::secrets::seal_or_block;

use crate::config::manifest::{ActorReference, EnvironmentsCategory};
use crate::github::Client;
use crate::github::actions::WriteOutcome;
use crate::github::environments::{EnvironmentReviewerInput, EnvironmentUpdate};

use super::*;

/// Resolve an [`ActorReference`] to the `(type, id)` pair GitHub's environment
/// reviewers field requires. Requires network access, so this only happens
/// at apply time (never during the sync `plan_environments_category` step).
async fn resolve_reviewer(
    client: &Client,
    actor: &ActorReference,
) -> Result<EnvironmentReviewerInput, String> {
    match actor {
        ActorReference::User { login } => client
            .get_user_by_login(login)
            .await
            .map(|user| EnvironmentReviewerInput {
                reviewer_type: "User",
                id: user.id,
            })
            .map_err(|_| format!("Could not resolve user `{login}` to an id")),
        ActorReference::Team { slug } => client
            .get_team_id(slug)
            .await
            .map(|id| EnvironmentReviewerInput {
                reviewer_type: "Team",
                id,
            })
            .map_err(|_| format!("Could not resolve team `{slug}` to an id")),
        other => Err(format!(
            "Actor kind `{other:?}` cannot be used as an environment reviewer (only users and teams are supported)"
        )),
    }
}

/// Apply a previously computed [`EnvironmentsPlan`].
pub async fn apply_environments_plan(
    client: &Client,
    repo: &str,
    plan: &EnvironmentsPlan,
) -> Result<EnvironmentsApplyResult> {
    let mut applied = Vec::new();
    let mut issues: Vec<ReconcileIssue> = plan
        .issues
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Blocker)
        .cloned()
        .collect();

    for env_plan in &plan.environment_plans {
        let scope_prefix = format!("environments.{}", env_plan.name);

        if let Some(settings) = &env_plan.settings_change {
            let mut reviewer_inputs = Vec::new();
            let mut reviewer_failed = false;
            for actor in &settings.reviewers {
                match resolve_reviewer(client, actor).await {
                    Ok(input) => reviewer_inputs.push(input),
                    Err(reason) => {
                        issues.push(ReconcileIssue::blocker(
                            format!("{scope_prefix}.reviewers"),
                            reason,
                        ));
                        reviewer_failed = true;
                    }
                }
            }

            if !reviewer_failed {
                let update = EnvironmentUpdate {
                    wait_timer: settings.wait_timer_minutes,
                    prevent_self_review: settings.prevent_self_review,
                    reviewers: Some(reviewer_inputs),
                    deployment_branch_policy: settings.deployment_branch_policy,
                };
                let outcome = client
                    .put_environment(repo, &env_plan.name, &update)
                    .await?;
                if let Some(issue) = write_outcome_issue(&scope_prefix, outcome, &mut applied) {
                    issues.push(issue);
                }
            }
        }

        for (pattern, policy_type) in &env_plan.branch_policy_creates {
            let scope = format!("{scope_prefix}.deployment_policy.{pattern}");
            match client
                .create_deployment_branch_policy(repo, &env_plan.name, pattern, policy_type)
                .await?
            {
                WriteOutcome::Applied(_) => applied.push(scope),
                WriteOutcome::Blocked(reason) => {
                    issues.push(ReconcileIssue::blocker(scope, reason))
                }
            }
        }
        for id in &env_plan.branch_policy_deletes {
            let scope = format!("{scope_prefix}.deployment_policy.{id}");
            let outcome = client
                .delete_deployment_branch_policy(repo, &env_plan.name, *id)
                .await?;
            if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
                issues.push(issue);
            }
        }

        if !env_plan.protection_app_enables.is_empty() {
            let available = client
                .list_available_deployment_protection_rule_apps(repo, &env_plan.name)
                .await
                .with_context(|| {
                    format!(
                        "Failed to list available deployment protection rule apps for `{}`",
                        env_plan.name
                    )
                })?;
            for slug in &env_plan.protection_app_enables {
                let scope = format!("{scope_prefix}.protection_apps.{slug}");
                match available.iter().find(|app| app.slug == *slug) {
                    Some(app) => match client.enable_deployment_protection_rule(repo, &env_plan.name, app.id).await? {
                        WriteOutcome::Applied(_) => applied.push(scope),
                        WriteOutcome::Blocked(reason) => issues.push(ReconcileIssue::blocker(scope, reason)),
                    },
                    None => issues.push(ReconcileIssue::blocker(
                        &scope,
                        format!("App `{slug}` is not installed/available as a deployment protection rule integration"),
                    )),
                }
            }
        }
        if !env_plan.protection_app_disables.is_empty() {
            // Only the app slug is known from the collected state; the
            // numeric protection-rule id required by the disable endpoint
            // must be resolved from the live rule list at apply time.
            let active_rules = client
                .list_deployment_protection_rules(repo, &env_plan.name)
                .await
                .with_context(|| {
                    format!(
                        "Failed to list active deployment protection rules for `{}`",
                        env_plan.name
                    )
                })?;
            for slug in &env_plan.protection_app_disables {
                let scope = format!("{scope_prefix}.protection_apps.{slug}");
                match active_rules.iter().find(|rule| rule.app.slug == *slug) {
                    Some(rule) => {
                        let outcome = client
                            .disable_deployment_protection_rule(repo, &env_plan.name, rule.id)
                            .await?;
                        if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
                            issues.push(issue);
                        }
                    }
                    None => applied.push(scope), // already absent; idempotent no-op
                }
            }
        }

        for variable in &env_plan.variable_upserts {
            let scope = format!("{scope_prefix}.variables.{}", variable.name);
            let existing = client
                .list_environment_variables(repo, &env_plan.name)
                .await?;
            let outcome = if existing.iter().any(|current| current.name == variable.name) {
                client
                    .update_environment_variable(
                        repo,
                        &env_plan.name,
                        &variable.name,
                        &variable.value,
                    )
                    .await?
            } else {
                client
                    .create_environment_variable(
                        repo,
                        &env_plan.name,
                        &variable.name,
                        &variable.value,
                    )
                    .await?
            };
            if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
                issues.push(issue);
            }
        }
        for name in &env_plan.variable_deletions {
            let scope = format!("{scope_prefix}.variables.{name}");
            let outcome = client
                .delete_environment_variable(repo, &env_plan.name, name)
                .await?;
            if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
                issues.push(issue);
            }
        }

        if !env_plan.secret_upserts.is_empty() {
            let public_key = client
                .get_environment_public_key(repo, &env_plan.name)
                .await
                .with_context(|| {
                    format!(
                        "Failed to fetch the secrets public key for environment `{}`",
                        env_plan.name
                    )
                })?;
            for secret in &env_plan.secret_upserts {
                let scope = format!("{scope_prefix}.secrets.{}", secret.name);
                match seal_or_block(&public_key.key, &secret.name, &secret.value) {
                    Ok(encrypted_value) => {
                        let outcome = client
                            .put_environment_secret(
                                repo,
                                &env_plan.name,
                                &secret.name,
                                &encrypted_value,
                                &public_key.key_id,
                            )
                            .await?;
                        if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
                            issues.push(issue);
                        }
                    }
                    Err(reason) => issues.push(ReconcileIssue::blocker(&scope, reason)),
                }
            }
        }
        for name in &env_plan.secret_deletions {
            let scope = format!("{scope_prefix}.secrets.{name}");
            let outcome = client
                .delete_environment_secret(repo, &env_plan.name, name)
                .await?;
            if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
                issues.push(issue);
            }
        }
    }

    for name in &plan.environment_deletions {
        let scope = format!("environments.{name}");
        let outcome = client.delete_environment(repo, name).await?;
        if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
            issues.push(issue);
        }
    }

    Ok(EnvironmentsApplyResult { applied, issues })
}

/// Re-collect and re-plan against `desired` to confirm convergence.
pub async fn verify_environments_category(
    client: &Client,
    repo: &str,
    desired: &EnvironmentsCategory,
) -> Result<EnvironmentsVerifyResult> {
    let actual = collect_environments_category(client, repo, Some(desired)).await?;
    let plan = plan_environments_category(desired, &actual);
    let compliant = !plan.has_actionable_changes() && !has_blocker(&plan.issues);
    Ok(EnvironmentsVerifyResult { compliant, plan })
}
