use std::collections::BTreeMap;

use crate::reconcile::common::issue::ReconcileIssue;
use crate::reconcile::common::secrets::resolve_secrets;
use crate::reconcile::common::secrets::{EnvLookup, process_env};

use crate::config::manifest::{
    ActorReference, EnvironmentConfig, EnvironmentsCategory, ManagementDisposition,
    SecretPlaceholderConfig,
};
use crate::github::environments::DeploymentBranchPolicySummary;

use super::*;

fn actor_key(actor: &ActorReference) -> String {
    match actor {
        ActorReference::OrganizationAdmin => "org-admin".to_owned(),
        ActorReference::Team { slug } => format!("team:{slug}"),
        ActorReference::User { login } => format!("user:{login}"),
        ActorReference::App { slug } => format!("app:{slug}"),
        ActorReference::Role { name } => format!("role:{name}"),
        ActorReference::Unresolved {
            actor_type,
            actor_id,
        } => {
            format!("unresolved:{actor_type}:{}", actor_id.unwrap_or_default())
        }
    }
}

/// Diff `desired` against a prior [`collect_environments_category`] observation.
pub fn plan_environments_category(
    desired: &EnvironmentsCategory,
    actual: &EnvironmentsCollection,
) -> EnvironmentsPlan {
    plan_environments_category_with_env(desired, actual, &process_env)
}

/// As [`plan_environments_category`], resolving secret values through `env`.
pub fn plan_environments_category_with_env(
    desired: &EnvironmentsCategory,
    actual: &EnvironmentsCollection,
    env: EnvLookup<'_>,
) -> EnvironmentsPlan {
    let mut issues = actual.issues.clone();

    if desired.policy.disposition != ManagementDisposition::Managed {
        return EnvironmentsPlan {
            environment_plans: Vec::new(),
            environment_deletions: Vec::new(),
            issues,
        };
    }

    let actual_by_name: BTreeMap<&str, &EnvironmentConfig> = actual
        .category
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();

    let mut environment_plans = Vec::new();
    let mut desired_names = std::collections::BTreeSet::new();

    for wanted in &desired.entries {
        desired_names.insert(wanted.name.as_str());
        let scope_prefix = format!("environments.{}", wanted.name);
        let current = actual_by_name.get(wanted.name.as_str()).copied();

        let mut plan = EnvironmentPlan {
            name: wanted.name.clone(),
            create: current.is_none(),
            ..EnvironmentPlan::default()
        };

        let current_reviewers: Vec<ActorReference> = current
            .map(|current| current.reviewers.iter().map(|r| r.actor.clone()).collect())
            .unwrap_or_default();
        let wanted_reviewers: Vec<ActorReference> =
            wanted.reviewers.iter().map(|r| r.actor.clone()).collect();
        let reviewers_differ = {
            let mut current_keys: Vec<String> = current_reviewers.iter().map(actor_key).collect();
            let mut wanted_keys: Vec<String> = wanted_reviewers.iter().map(actor_key).collect();
            current_keys.sort();
            wanted_keys.sort();
            current_keys != wanted_keys
        };

        let current_wait_timer = current.and_then(|current| current.wait_timer_minutes);
        let current_prevent_self_review = current.and_then(|current| current.prevent_self_review);
        let current_branch_policy_summary = current
            .and_then(|current| current.deployment_policy.as_ref())
            .and_then(|policy| {
                Some(DeploymentBranchPolicySummary {
                    protected_branches: policy.protected_branches?,
                    custom_branch_policies: policy.custom_branch_policies.unwrap_or(false),
                })
            });

        // A settings update is warranted whenever either branch-policy flag
        // is explicitly set. GitHub treats `protected_branches` and
        // `custom_branch_policies` as mutually exclusive, so an omitted
        // `protected_branches` defaults to `false`. This matters when
        // `custom_branch_policies = true` is set without `protected_branches`:
        // the environment must first be updated to `{protected_branches:
        // false, custom_branch_policies: true}` before any custom pattern can
        // be created.
        let wanted_branch_policy_summary = wanted.deployment_policy.as_ref().and_then(|policy| {
            match (policy.protected_branches, policy.custom_branch_policies) {
                (None, None) => None,
                (protected, custom) => Some(DeploymentBranchPolicySummary {
                    protected_branches: protected.unwrap_or(false),
                    custom_branch_policies: custom.unwrap_or(false),
                }),
            }
        });

        let wait_timer_changed = plan.create
            || (wanted.wait_timer_minutes.is_some()
                && wanted.wait_timer_minutes != current_wait_timer);
        let prevent_self_review_changed = plan.create
            || (wanted.prevent_self_review.is_some()
                && wanted.prevent_self_review != current_prevent_self_review);
        let branch_policy_summary_changed = plan.create
            || (wanted_branch_policy_summary.is_some()
                && wanted_branch_policy_summary != current_branch_policy_summary);

        if wait_timer_changed
            || prevent_self_review_changed
            || reviewers_differ
            || branch_policy_summary_changed
        {
            plan.settings_change = Some(EnvironmentSettingsChange {
                wait_timer_minutes: wanted.wait_timer_minutes.or(current_wait_timer),
                prevent_self_review: wanted.prevent_self_review.or(current_prevent_self_review),
                reviewers: wanted_reviewers,
                deployment_branch_policy: wanted_branch_policy_summary
                    .or(current_branch_policy_summary),
            });
        }

        // Deployment branch/tag policy patterns.
        if let Some(policy) = &wanted.deployment_policy {
            let custom_allowed = policy.custom_branch_policies.unwrap_or(false);
            if (!policy.branch_patterns.is_empty() || !policy.tag_patterns.is_empty())
                && !custom_allowed
            {
                issues.push(ReconcileIssue::warning(
                    format!("{scope_prefix}.deployment_policy"),
                    "Branch/tag patterns were specified but `custom_branch_policies` is not enabled; GitHub will ignore them",
                ));
            } else {
                let current_branch_patterns: Vec<&str> = current
                    .and_then(|current| current.deployment_policy.as_ref())
                    .map(|policy| policy.branch_patterns.iter().map(String::as_str).collect())
                    .unwrap_or_default();
                let current_tag_patterns: Vec<&str> = current
                    .and_then(|current| current.deployment_policy.as_ref())
                    .map(|policy| policy.tag_patterns.iter().map(String::as_str).collect())
                    .unwrap_or_default();

                for pattern in &policy.branch_patterns {
                    if !current_branch_patterns.contains(&pattern.as_str()) {
                        plan.branch_policy_creates
                            .push((pattern.clone(), "branch".to_owned()));
                    }
                }
                for pattern in &policy.tag_patterns {
                    if !current_tag_patterns.contains(&pattern.as_str()) {
                        plan.branch_policy_creates
                            .push((pattern.clone(), "tag".to_owned()));
                    }
                }
            }
        }

        // Prune current branch/tag patterns that are absent from desired.
        // Requires a complete observation of this environment's
        // deployment-branch-policy listing: a coverage-gapped read must never
        // delete. Patterns are matched by both type and name, resolved to the
        // numeric GitHub id retained in collection state.
        if desired.policy.prune
            && actual
                .deployment_policies_observed
                .contains(wanted.name.as_str())
        {
            let desired_branch_patterns: std::collections::BTreeSet<&str> = wanted
                .deployment_policy
                .as_ref()
                .map(|policy| policy.branch_patterns.iter().map(String::as_str).collect())
                .unwrap_or_default();
            let desired_tag_patterns: std::collections::BTreeSet<&str> = wanted
                .deployment_policy
                .as_ref()
                .map(|policy| policy.tag_patterns.iter().map(String::as_str).collect())
                .unwrap_or_default();

            if let Some(current_policy) =
                current.and_then(|current| current.deployment_policy.as_ref())
            {
                for (patterns, policy_type, desired_patterns) in [
                    (
                        &current_policy.branch_patterns,
                        "branch",
                        &desired_branch_patterns,
                    ),
                    (&current_policy.tag_patterns, "tag", &desired_tag_patterns),
                ] {
                    for name in patterns {
                        if desired_patterns.contains(name.as_str()) {
                            continue;
                        }
                        if let Some(id) = actual.deployment_policy_ids.get(&(
                            wanted.name.clone(),
                            policy_type.to_owned(),
                            name.clone(),
                        )) {
                            plan.branch_policy_deletes.push(*id);
                        }
                    }
                }
            }
        }

        // Protection apps (custom deployment protection rules), by slug.
        let current_apps: Vec<&str> = current
            .map(|current| {
                current
                    .protection_apps
                    .iter()
                    .map(|app| app.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        for app in &wanted.protection_apps {
            if !current_apps.contains(&app.name.as_str()) {
                plan.protection_app_enables.push(app.name.clone());
            }
        }
        if desired.policy.prune {
            let wanted_apps: std::collections::BTreeSet<&str> = wanted
                .protection_apps
                .iter()
                .map(|app| app.name.as_str())
                .collect();
            for slug in &current_apps {
                if !wanted_apps.contains(slug) {
                    plan.protection_app_disables.push((*slug).to_owned());
                }
            }
        }

        // Environment variables: full diff.
        let current_variables: BTreeMap<&str, &str> = current
            .map(|current| {
                current
                    .variables
                    .iter()
                    .map(|v| (v.name.as_str(), v.value.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let mut desired_variable_names = std::collections::BTreeSet::new();
        for variable in &wanted.variables {
            desired_variable_names.insert(variable.name.as_str());
            if current_variables.get(variable.name.as_str()) != Some(&variable.value.as_str()) {
                plan.variable_upserts.push(variable.clone());
            }
        }
        if desired.policy.prune {
            for name in current_variables.keys() {
                if !desired_variable_names.contains(name) {
                    plan.variable_deletions.push((*name).to_owned());
                }
            }
        }

        // Environment secrets: idempotent by name, same rationale as Actions
        // secrets (see `plan_actions_category`). Only names absent from the
        // last observation are resolved/upserted.
        let current_secret_names: std::collections::BTreeSet<&str> = current
            .map(|current| {
                current
                    .secrets
                    .iter()
                    .map(|secret| secret.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let missing_secrets: Vec<SecretPlaceholderConfig> = wanted
            .secrets
            .iter()
            .filter(|secret| !current_secret_names.contains(secret.name.as_str()))
            .cloned()
            .collect();
        plan.secret_upserts = resolve_secrets(&missing_secrets, &scope_prefix, &mut issues, env);
        if desired.policy.prune {
            let desired_secret_names: std::collections::BTreeSet<&str> = wanted
                .secrets
                .iter()
                .map(|secret| secret.name.as_str())
                .collect();
            if let Some(current) = current {
                for secret in &current.secrets {
                    if !desired_secret_names.contains(secret.name.as_str()) {
                        plan.secret_deletions.push(secret.name.clone());
                    }
                }
            }
        }

        environment_plans.push(plan);
    }

    let mut environment_deletions = Vec::new();
    if desired.policy.prune {
        for name in &actual.observed_names {
            if !desired_names.contains(name.as_str()) {
                environment_deletions.push(name.clone());
            }
        }
    }

    EnvironmentsPlan {
        environment_plans,
        environment_deletions,
        issues,
    }
}
