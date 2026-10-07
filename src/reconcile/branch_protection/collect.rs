//! Branch protection state collection.

use std::collections::HashMap;

use anyhow::Result;

use crate::config::manifest::{
    ActorReference, BranchProtectionCategory, BranchProtectionConfig, BranchStatusCheckConfig,
    CategoryPolicy, DetailedBranchProtectionConfig, ManifestCategoryName, ProtectedBranchConfig,
};
use crate::github::Client;
use crate::github::branch_protection::{
    ActorSet, DetailedBranchProtection, StatusCheckRequirement,
};
use crate::reconcile::common::actors::actor_reference_key;
use crate::reconcile::common::coverage::{
    collected_entry, not_applicable_entry, permission_denied_entry, unavailable_entry,
};

use super::*;

pub async fn collect_branch_protection_category(
    client: &Client,
    repo: &str,
    category: Option<&BranchProtectionCategory>,
) -> Result<BranchProtectionCollection> {
    let repository = client.get_repo(repo).await?;
    collect_branch_protection_category_for_branch(client, repo, repository.default_branch, category)
        .await
}

/// Collect branch protection when the default branch is already known.
pub async fn collect_branch_protection_category_for_branch(
    client: &Client,
    repo: &str,
    default_branch_name: String,
    category: Option<&BranchProtectionCategory>,
) -> Result<BranchProtectionCollection> {
    let branches = client.list_protected_branches(repo).await?;

    let issues = Vec::new();
    let mut coverage = vec![
        collected_entry(
            ManifestCategoryName::BranchProtection,
            "GET /repos/{owner}/{repo}",
        ),
        collected_entry(
            ManifestCategoryName::BranchProtection,
            "GET /repos/{owner}/{repo}/branches?protected=true",
        ),
    ];
    let mut actual_branches = Vec::new();
    let app_slugs_by_id: HashMap<i64, String> = match client.list_org_installations().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::BranchProtection,
                "GET /orgs/{org}/installations",
            ));
            value
                .into_iter()
                .map(|app| (app.app_id as i64, app.app_slug))
                .collect()
        }
        // User-owned accounts have no organization installations endpoint.
        Err(error) if crate::github::is_not_found(&error) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::BranchProtection,
                "GET /orgs/{org}/installations",
                "the owner is not an organization, so there are no organization app installations"
                    .to_owned(),
            ));
            HashMap::new()
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::BranchProtection,
                "GET /orgs/{org}/installations",
                format!("{error:#}"),
            ));
            HashMap::new()
        }
    };

    for branch in branches {
        let detail = match client
            .read_branch_protection_detail(repo, &branch.name)
            .await?
        {
            crate::github::actions::ReadOutcome::Available(detail) => {
                coverage.push(collected_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                ));
                detail
            }
            crate::github::actions::ReadOutcome::NotApplicable(reason) => {
                coverage.push(not_applicable_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                    reason,
                ));
                continue;
            }
            crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
                coverage.push(permission_denied_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                    reason,
                ));
                continue;
            }
            crate::github::actions::ReadOutcome::Unavailable(reason) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                    reason,
                ));
                continue;
            }
        };

        let manifest = protected_branch_from_detail(&branch.name, &detail, &app_slugs_by_id);
        let status_checks = detail
            .required_status_checks
            .as_ref()
            .map_or_else(Vec::new, |value| {
                if value.checks.is_empty() {
                    value
                        .contexts
                        .iter()
                        .map(|context| StatusCheckRequirement {
                            context: context.clone(),
                            app_id: None,
                        })
                        .collect::<Vec<_>>()
                } else {
                    value.checks.clone()
                }
            });

        actual_branches.push(ActualProtectedBranch {
            name: branch.name.clone(),
            is_default_branch: branch.name == default_branch_name,
            manifest,
            raw: detail,
            status_checks,
        });
    }

    actual_branches.sort_by(|left, right| left.name.cmp(&right.name));

    let default_branch = actual_branches
        .iter()
        .find(|branch| branch.is_default_branch)
        .map(|branch| branch.manifest.protection.clone());
    let default_branch_detailed = actual_branches
        .iter()
        .find(|branch| branch.is_default_branch)
        .map(|branch| detailed_branch_config_from_manifest(&branch.manifest));
    let protected_branches = actual_branches
        .iter()
        .filter(|branch| !branch.is_default_branch)
        .map(|branch| branch.manifest.clone())
        .collect();

    let policy = category
        .map(|value| value.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    Ok(BranchProtectionCollection {
        default_branch_name,
        category: BranchProtectionCategory {
            policy,
            default_branch,
            default_branch_detailed,
            protected_branches,
        },
        actual_branches,
        app_ids_by_slug: app_slugs_by_id
            .iter()
            .map(|(id, slug)| (slug.clone(), *id))
            .collect(),
        app_slugs_by_id,
        coverage,
        issues,
    })
}

fn protected_branch_from_detail(
    name: &str,
    detail: &DetailedBranchProtection,
    app_slugs_by_id: &HashMap<i64, String>,
) -> ProtectedBranchConfig {
    let status_check_contexts = detail
        .required_status_checks
        .as_ref()
        .map(|checks| {
            if checks.contexts.is_empty() {
                checks
                    .checks
                    .iter()
                    .map(|check| check.context.clone())
                    .collect::<Vec<_>>()
            } else {
                checks.contexts.clone()
            }
        })
        .unwrap_or_default();
    let status_checks = detail
        .required_status_checks
        .as_ref()
        .map_or_else(Vec::new, |checks| {
            if checks.checks.is_empty() {
                checks
                    .contexts
                    .iter()
                    .map(|context| BranchStatusCheckConfig {
                        context: context.clone(),
                        app_id: None,
                        app_slug: None,
                    })
                    .collect()
            } else {
                checks
                    .checks
                    .iter()
                    .map(|check| BranchStatusCheckConfig {
                        context: check.context.clone(),
                        app_id: check.app_id,
                        app_slug: check
                            .app_id
                            .and_then(|id| app_slugs_by_id.get(&id).cloned()),
                    })
                    .collect()
            }
        });

    ProtectedBranchConfig {
        name: name.to_owned(),
        protection: BranchProtectionConfig {
            enabled: detail.required_pull_request_reviews.is_some(),
            required_approvals: detail
                .required_pull_request_reviews
                .as_ref()
                .map(|reviews| reviews.required_approving_review_count)
                .unwrap_or(0),
            dismiss_stale_reviews: detail
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|reviews| reviews.dismiss_stale_reviews),
            require_code_owner_reviews: detail
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|reviews| reviews.require_code_owner_reviews),
            require_status_checks: detail.required_status_checks.is_some(),
            strict_status_checks: detail
                .required_status_checks
                .as_ref()
                .is_some_and(|checks| checks.strict),
            enforce_admins: detail
                .enforce_admins
                .as_ref()
                .is_some_and(|value| value.enabled),
            required_linear_history: detail
                .required_linear_history
                .as_ref()
                .is_some_and(|value| value.enabled),
            allow_force_pushes: detail
                .allow_force_pushes
                .as_ref()
                .is_some_and(|value| value.enabled),
            allow_deletions: detail
                .allow_deletions
                .as_ref()
                .is_some_and(|value| value.enabled),
        },
        status_check_contexts,
        status_checks,
        push_restrictions: actor_set_to_references(
            detail.restrictions.as_ref().cloned().unwrap_or_default(),
        ),
        dismissal_restrictions: detail
            .required_pull_request_reviews
            .as_ref()
            .map(|reviews| actor_set_to_references(reviews.dismissal_restrictions.clone()))
            .unwrap_or_default(),
        pull_request_bypass_allowances: detail
            .required_pull_request_reviews
            .as_ref()
            .map(|reviews| actor_set_to_references(reviews.bypass_pull_request_allowances.clone()))
            .unwrap_or_default(),
        require_last_push_approval: detail
            .required_pull_request_reviews
            .as_ref()
            .and_then(|reviews| reviews.require_last_push_approval),
        block_creations: detail.block_creations.as_ref().map(|value| value.enabled),
        required_reviewers: detail
            .required_pull_request_reviews
            .as_ref()
            .and_then(|reviews| reviews.required_reviewers.clone()),
        require_conversation_resolution: detail
            .required_conversation_resolution
            .as_ref()
            .map(|value| value.enabled),
        require_signed_commits: detail
            .required_signatures
            .as_ref()
            .map(|value| value.enabled),
        lock_branch: detail.lock_branch.as_ref().map(|value| value.enabled),
        allow_fork_syncing: detail
            .allow_fork_syncing
            .as_ref()
            .map(|value| value.enabled),
    }
}

fn detailed_branch_config_from_manifest(
    branch: &ProtectedBranchConfig,
) -> DetailedBranchProtectionConfig {
    DetailedBranchProtectionConfig {
        protection: branch.protection.clone(),
        status_check_contexts: branch.status_check_contexts.clone(),
        status_checks: branch.status_checks.clone(),
        push_restrictions: branch.push_restrictions.clone(),
        dismissal_restrictions: branch.dismissal_restrictions.clone(),
        pull_request_bypass_allowances: branch.pull_request_bypass_allowances.clone(),
        require_last_push_approval: branch.require_last_push_approval,
        block_creations: branch.block_creations,
        required_reviewers: branch.required_reviewers.clone(),
        require_conversation_resolution: branch.require_conversation_resolution,
        require_signed_commits: branch.require_signed_commits,
        lock_branch: branch.lock_branch,
        allow_fork_syncing: branch.allow_fork_syncing,
    }
}

fn actor_set_to_references(actors: ActorSet) -> Vec<ActorReference> {
    let mut references = actors
        .users
        .into_iter()
        .map(|user| ActorReference::User { login: user.login })
        .chain(
            actors
                .teams
                .into_iter()
                .map(|team| ActorReference::Team { slug: team.slug }),
        )
        .chain(
            actors
                .apps
                .into_iter()
                .map(|app| ActorReference::App { slug: app.slug }),
        )
        .collect::<Vec<_>>();
    references.sort_by_key(actor_reference_key);
    references
}
