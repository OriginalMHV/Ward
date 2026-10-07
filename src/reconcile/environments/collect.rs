use std::collections::BTreeMap;

use anyhow::{Context, Result};

use crate::reconcile::common::coverage::record_read_outcome;

use crate::config::manifest::{
    ActorReference, CategoryPolicy, EnvironmentConfig, EnvironmentDeploymentPolicyConfig,
    EnvironmentsCategory, ExternalValueReference, ManifestCategoryName, NamedValueConfig,
    ReferencedResourceConfig, ReferencedResourceType, SecretPlaceholderConfig,
};
use crate::github::Client;
use crate::github::environments::{self};

use super::*;

fn reviewer_actor_from(reviewer: &environments::ProtectionRuleReviewer) -> Option<ActorReference> {
    match reviewer.reviewer_type.as_str() {
        "User" => reviewer
            .reviewer
            .login
            .clone()
            .map(|login| ActorReference::User { login }),
        "Team" => reviewer
            .reviewer
            .slug
            .clone()
            .map(|slug| ActorReference::Team { slug }),
        other => Some(ActorReference::Unresolved {
            actor_type: other.to_owned(),
            actor_id: reviewer.reviewer.id,
        }),
    }
}

/// Extract environment reviewers and `prevent_self_review` from a
/// `RequiredReviewers` protection rule. GitHub does not surface these as
/// top-level `Environment` fields in practice (only `protection_rules`
/// carries them), so this must always be consulted rather than a top-level
/// field.
fn reviewers_from_protection_rules(
    env: &environments::Environment,
) -> (
    Option<bool>,
    Vec<crate::config::manifest::EnvironmentReviewerConfig>,
) {
    match env.required_reviewers() {
        Some((prevent_self_review, reviewers)) => {
            let reviewers = reviewers
                .iter()
                .filter_map(reviewer_actor_from)
                .map(|actor| crate::config::manifest::EnvironmentReviewerConfig { actor })
                .collect();
            (Some(prevent_self_review), reviewers)
        }
        None => (None, Vec::new()),
    }
}

/// Collect the observed environments for `repo`. When `desired` is provided,
/// only its named environments are resolved further (variables/secrets/branch
/// policies/protection apps); otherwise only the top-level environment list
/// (name + settings visible on the environments list endpoint) is returned.
///
/// Every optional sub-endpoint (branch policies, protection rules,
/// variables, secrets) is read through a `_checked` client method: a
/// 403/404/422 on any single environment's sub-endpoint is recorded as a
/// [`CoverageEntry`] and does not abort collection of the remaining
/// environments.
pub async fn collect_environments_category(
    client: &Client,
    repo: &str,
    desired: Option<&EnvironmentsCategory>,
) -> Result<EnvironmentsCollection> {
    let mut issues = Vec::new();
    let mut coverage = Vec::new();
    let mut deployment_policy_ids: BTreeMap<(String, String, String), u64> = BTreeMap::new();
    let mut deployment_policies_observed: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    // A collected/observed snapshot must carry a sensible policy: preserve
    // the caller's `desired.policy` when reconciling against a manifest, or
    // default to `observe_sensitive()` for a bare import snapshot (never
    // silently downgrade to a plain, non-sensitive `Observe` default that
    // would misrepresent environment secrets/reviewers as low-sensitivity).
    let policy = desired
        .map(|desired| desired.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    let Some(observed) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Environments,
        "environments",
        client
            .list_environments_checked(repo)
            .await
            .context("Failed to list repository environments")?,
    ) else {
        return Ok(EnvironmentsCollection {
            category: EnvironmentsCategory {
                policy,
                ..EnvironmentsCategory::default()
            },
            observed_names: Vec::new(),
            deployment_policy_ids: BTreeMap::new(),
            deployment_policies_observed: std::collections::BTreeSet::new(),
            coverage,
            issues,
        });
    };

    let wanted_names: Option<std::collections::BTreeSet<&str>> = desired.map(|desired| {
        desired
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    });

    let mut entries = Vec::new();
    for env in &observed {
        if let Some(names) = &wanted_names
            && !names.contains(env.name.as_str())
        {
            continue;
        }

        let (prevent_self_review, reviewers) = reviewers_from_protection_rules(env);
        let wait_timer_minutes = env.wait_timer_minutes();

        let mut deployment_policy =
            env.deployment_branch_policy
                .map(|summary| EnvironmentDeploymentPolicyConfig {
                    protected_branches: Some(summary.protected_branches),
                    custom_branch_policies: Some(summary.custom_branch_policies),
                    branch_patterns: Vec::new(),
                    tag_patterns: Vec::new(),
                });

        if let Some(branch_policies) = record_read_outcome(
            &mut coverage,
            ManifestCategoryName::Environments,
            &format!("environments/{}/deployment-branch-policies", env.name),
            client
                .list_deployment_branch_policies_checked(repo, &env.name)
                .await
                .with_context(|| {
                    format!(
                        "Failed to list deployment branch policies for environment `{}`",
                        env.name
                    )
                })?,
        ) {
            // A complete read (even an empty one) makes this environment
            // eligible for pattern prune; a coverage-gapped read must not.
            deployment_policies_observed.insert(env.name.clone());
            if !branch_policies.is_empty() {
                let policy = deployment_policy
                    .get_or_insert_with(EnvironmentDeploymentPolicyConfig::default);
                for branch_policy in &branch_policies {
                    let policy_type = match branch_policy.policy_type.as_str() {
                        "tag" => {
                            policy.tag_patterns.push(branch_policy.name.clone());
                            "tag"
                        }
                        _ => {
                            policy.branch_patterns.push(branch_policy.name.clone());
                            "branch"
                        }
                    };
                    deployment_policy_ids.insert(
                        (
                            env.name.clone(),
                            policy_type.to_owned(),
                            branch_policy.name.clone(),
                        ),
                        branch_policy.id,
                    );
                }
            }
        }

        let protection_apps = record_read_outcome(
            &mut coverage,
            ManifestCategoryName::Environments,
            &format!("environments/{}/deployment_protection_rules", env.name),
            client
                .list_deployment_protection_rules_checked(repo, &env.name)
                .await
                .with_context(|| {
                    format!(
                        "Failed to list deployment protection rules for environment `{}`",
                        env.name
                    )
                })?,
        )
        .map(|rules| {
            rules
                .into_iter()
                .map(|rule| ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::App,
                    name: rule.app.slug,
                })
                .collect()
        })
        .unwrap_or_default();

        let variables = record_read_outcome(
            &mut coverage,
            ManifestCategoryName::Environments,
            &format!("environments/{}/variables", env.name),
            client
                .list_environment_variables_checked(repo, &env.name)
                .await
                .with_context(|| {
                    format!("Failed to list variables for environment `{}`", env.name)
                })?,
        )
        .map(|variables| {
            variables
                .into_iter()
                .map(|variable| NamedValueConfig {
                    name: variable.name,
                    value: variable.value,
                })
                .collect()
        })
        .unwrap_or_default();

        let secrets = record_read_outcome(
            &mut coverage,
            ManifestCategoryName::Environments,
            &format!("environments/{}/secrets", env.name),
            client
                .list_environment_secrets_checked(repo, &env.name)
                .await
                .with_context(|| {
                    format!(
                        "Failed to list secret metadata for environment `{}`",
                        env.name
                    )
                })?,
        )
        .map(|secrets| {
            secrets
                .into_iter()
                .map(|secret| SecretPlaceholderConfig {
                    name: secret.name,
                    value_from: ExternalValueReference::Manual {
                        hint: Some(
                            "Existing secret; GitHub never returns secret values".to_owned(),
                        ),
                    },
                })
                .collect()
        })
        .unwrap_or_default();

        entries.push(EnvironmentConfig {
            name: env.name.clone(),
            wait_timer_minutes,
            prevent_self_review,
            deployment_policy,
            reviewers,
            protection_apps,
            variables,
            secrets,
        });
    }

    Ok(EnvironmentsCollection {
        category: EnvironmentsCategory { policy, entries },
        observed_names: observed.iter().map(|env| env.name.clone()).collect(),
        deployment_policy_ids,
        deployment_policies_observed,
        coverage,
        issues: std::mem::take(&mut issues),
    })
}
