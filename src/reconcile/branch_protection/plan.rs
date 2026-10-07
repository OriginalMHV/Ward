//! Branch protection plan computation.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};

use crate::config::manifest::{
    ActorReference, BranchProtectionCategory, BranchProtectionConfig, BranchStatusCheckConfig,
    DetailedBranchProtectionConfig, ManagementDisposition, ProtectedBranchConfig,
};
use crate::github::branch_protection::{
    ActorSet, AppActor, DesiredBranchProtection, StatusCheckRequirement, TeamActor, UserActor,
};
use crate::reconcile::common::actors::{actor_reference_key, normalize_actor_refs};
use crate::reconcile::common::rules_issue::ReconcileIssue;
use crate::reconcile::common::rules_issue::blocker_issue;

use super::*;

pub fn plan_branch_protection_category(
    desired: &BranchProtectionCategory,
    actual: &BranchProtectionCollection,
) -> Result<BranchProtectionPlan> {
    let mut issues = actual.issues.clone();
    let mut actions = Vec::new();

    let actual_by_name: BTreeMap<&str, &ActualProtectedBranch> = actual
        .actual_branches
        .iter()
        .map(|branch| (branch.name.as_str(), branch))
        .collect();
    let mut desired_by_name: BTreeMap<String, DesiredBranchProtection> = BTreeMap::new();

    if desired.policy.disposition != ManagementDisposition::Managed {
        for branch in &actual.actual_branches {
            actions.push(BranchProtectionPlanAction::Unchanged {
                branch: branch.name.clone(),
            });
        }
        return Ok(BranchProtectionPlan { actions, issues });
    }

    if let Some(config) = &desired.default_branch_detailed {
        let existing = actual_by_name
            .get(actual.default_branch_name.as_str())
            .copied();
        let plan = desired_branch_protection_from_detailed_manifest(
            &actual.default_branch_name,
            config,
            existing,
            &actual.app_ids_by_slug,
            &mut issues,
        )?;
        desired_by_name.insert(actual.default_branch_name.clone(), plan);
    } else if let Some(config) = &desired.default_branch {
        let existing = actual_by_name
            .get(actual.default_branch_name.as_str())
            .copied();
        let plan = desired_branch_protection_from_default(
            &actual.default_branch_name,
            config,
            existing,
            &actual.app_ids_by_slug,
            &mut issues,
        )?;
        desired_by_name.insert(actual.default_branch_name.clone(), plan);
    }
    for protected_branch in &desired.protected_branches {
        let existing = actual_by_name.get(protected_branch.name.as_str()).copied();
        let plan = desired_branch_protection_from_manifest(
            protected_branch,
            existing,
            &actual.app_ids_by_slug,
            &mut issues,
        )?;
        desired_by_name.insert(protected_branch.name.clone(), plan);
    }

    for (branch, desired_branch) in &desired_by_name {
        if let Some(existing) = actual_by_name.get(branch.as_str()) {
            if protected_branch_matches(&existing.manifest, desired_branch) {
                actions.push(BranchProtectionPlanAction::Unchanged {
                    branch: branch.clone(),
                });
            } else {
                actions.push(BranchProtectionPlanAction::Upsert {
                    branch: branch.clone(),
                    desired: Box::new(desired_branch.clone()),
                });
            }
        } else {
            actions.push(BranchProtectionPlanAction::Upsert {
                branch: branch.clone(),
                desired: Box::new(desired_branch.clone()),
            });
        }
    }

    if desired.policy.prune {
        for branch in &actual.actual_branches {
            if !desired_by_name.contains_key(&branch.name) {
                actions.push(BranchProtectionPlanAction::Delete {
                    branch: branch.name.clone(),
                });
            }
        }
    }

    actions.sort_by_key(branch_action_sort_key);

    if actions
        .iter()
        .any(|action| !matches!(action, BranchProtectionPlanAction::Unchanged { .. }))
        && !desired.policy.sensitive
    {
        issues.push(blocker_issue(
            Some("categories.branch_protection.policy.sensitive".to_owned()),
            "branch-protection-sensitive-gate",
            "Managing legacy branch protection requires policy.sensitive = true".to_owned(),
        ));
    }

    Ok(BranchProtectionPlan { actions, issues })
}

fn desired_branch_protection_from_default(
    branch_name: &str,
    config: &BranchProtectionConfig,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    let preserved = existing.map(|branch| &branch.manifest);
    desired_branch_protection_from_parts(
        branch_name,
        config,
        preserved
            .map(|branch| branch.status_check_contexts.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.status_checks.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.push_restrictions.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.dismissal_restrictions.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.pull_request_bypass_allowances.as_slice())
            .unwrap_or(&[]),
        existing.and_then(|branch| {
            branch
                .raw
                .required_pull_request_reviews
                .as_ref()
                .and_then(|reviews| reviews.require_last_push_approval)
        }),
        existing
            .and_then(|branch| branch.raw.block_creations.as_ref())
            .map(|value| value.enabled),
        existing
            .and_then(|branch| branch.raw.required_pull_request_reviews.as_ref())
            .and_then(|reviews| reviews.required_reviewers.clone()),
        preserved.and_then(|branch| branch.require_conversation_resolution),
        preserved.and_then(|branch| branch.require_signed_commits),
        preserved.and_then(|branch| branch.lock_branch),
        preserved.and_then(|branch| branch.allow_fork_syncing),
        existing,
        app_ids_by_slug,
        issues,
    )
}

fn desired_branch_protection_from_detailed_manifest(
    branch_name: &str,
    config: &DetailedBranchProtectionConfig,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    desired_branch_protection_from_parts(
        branch_name,
        &config.protection,
        config.status_check_contexts.as_slice(),
        config.status_checks.as_slice(),
        config.push_restrictions.as_slice(),
        config.dismissal_restrictions.as_slice(),
        config.pull_request_bypass_allowances.as_slice(),
        config.require_last_push_approval,
        config.block_creations,
        config.required_reviewers.clone(),
        config.require_conversation_resolution,
        config.require_signed_commits,
        config.lock_branch,
        config.allow_fork_syncing,
        existing,
        app_ids_by_slug,
        issues,
    )
}

fn desired_branch_protection_from_manifest(
    protected_branch: &ProtectedBranchConfig,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    desired_branch_protection_from_parts(
        &protected_branch.name,
        &protected_branch.protection,
        protected_branch.status_check_contexts.as_slice(),
        protected_branch.status_checks.as_slice(),
        protected_branch.push_restrictions.as_slice(),
        protected_branch.dismissal_restrictions.as_slice(),
        protected_branch.pull_request_bypass_allowances.as_slice(),
        protected_branch.require_last_push_approval,
        protected_branch.block_creations,
        protected_branch.required_reviewers.clone(),
        protected_branch.require_conversation_resolution,
        protected_branch.require_signed_commits,
        protected_branch.lock_branch,
        protected_branch.allow_fork_syncing,
        existing,
        app_ids_by_slug,
        issues,
    )
}

#[allow(clippy::too_many_arguments)]
fn desired_branch_protection_from_parts(
    branch_name: &str,
    config: &BranchProtectionConfig,
    status_check_contexts: &[String],
    status_checks_config: &[BranchStatusCheckConfig],
    push_restrictions: &[ActorReference],
    dismissal_restrictions: &[ActorReference],
    pull_request_bypass_allowances: &[ActorReference],
    require_last_push_approval: Option<bool>,
    block_creations: Option<bool>,
    required_reviewers: Option<serde_json::Value>,
    require_conversation_resolution: Option<bool>,
    require_signed_commits: Option<bool>,
    lock_branch: Option<bool>,
    allow_fork_syncing: Option<bool>,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    let push_restrictions = actor_refs_to_actor_set(branch_name, push_restrictions, issues);
    let dismissal_restrictions =
        actor_refs_to_actor_set(branch_name, dismissal_restrictions, issues);
    let pull_request_bypass_allowances =
        actor_refs_to_actor_set(branch_name, pull_request_bypass_allowances, issues);
    let desired_status_checks = desired_status_checks(
        branch_name,
        status_check_contexts,
        status_checks_config,
        existing,
        app_ids_by_slug,
        issues,
    )?;

    Ok(DesiredBranchProtection {
        required_pull_request_reviews: config.enabled,
        required_approving_review_count: config.required_approvals,
        dismiss_stale_reviews: config.dismiss_stale_reviews,
        require_code_owner_reviews: config.require_code_owner_reviews,
        require_last_push_approval,
        required_status_checks: config.require_status_checks,
        strict_status_checks: config.strict_status_checks,
        status_check_contexts: status_check_contexts.to_vec(),
        status_checks: desired_status_checks,
        push_restrictions,
        dismissal_restrictions,
        pull_request_bypass_allowances,
        enforce_admins: config.enforce_admins,
        required_linear_history: config.required_linear_history,
        allow_force_pushes: config.allow_force_pushes,
        allow_deletions: config.allow_deletions,
        block_creations,
        require_conversation_resolution,
        require_signed_commits,
        lock_branch,
        allow_fork_syncing,
        required_reviewers,
    })
}

fn desired_status_checks(
    branch_name: &str,
    status_check_contexts: &[String],
    status_checks_config: &[BranchStatusCheckConfig],
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<Vec<StatusCheckRequirement>> {
    if !status_checks_config.is_empty() {
        return status_checks_config
            .iter()
            .map(|check| {
                let app_id = match (&check.app_slug, check.app_id) {
                    (Some(slug), _) => Some(*app_ids_by_slug.get(slug).with_context(|| {
                        format!(
                            "Status check {} on {} references unknown GitHub App slug {}",
                            check.context, branch_name, slug
                        )
                    })?),
                    (None, Some(app_id)) => {
                        issues.push(blocker_issue(
                            Some(branch_name.to_owned()),
                            "branch-protection-unresolved-status-check-app",
                            format!(
                                "Status check {} on {} is pinned to app_id {} without a stable app_slug; refusing to apply unresolved app bindings.",
                                check.context, branch_name, app_id
                            ),
                        ));
                        Some(app_id)
                    }
                    (None, None) => None,
                };
                Ok(StatusCheckRequirement {
                    context: check.context.clone(),
                    app_id,
                })
            })
            .collect();
    }

    if let Some(existing) = existing {
        let actual_contexts = existing
            .status_checks
            .iter()
            .map(|check| check.context.clone())
            .collect::<Vec<_>>();
        if actual_contexts == status_check_contexts {
            return Ok(existing.status_checks.clone());
        }
    }

    Ok(status_check_contexts
        .iter()
        .map(|context| StatusCheckRequirement {
            context: context.clone(),
            app_id: None,
        })
        .collect())
}

fn actor_refs_to_actor_set(
    branch_name: &str,
    actors: &[ActorReference],
    issues: &mut Vec<ReconcileIssue>,
) -> ActorSet {
    let mut set = ActorSet::default();
    for actor in actors {
        match actor {
            ActorReference::User { login } => set.users.push(UserActor {
                login: login.clone(),
            }),
            ActorReference::Team { slug } => set.teams.push(TeamActor { slug: slug.clone() }),
            ActorReference::App { slug } => set.apps.push(AppActor { slug: slug.clone() }),
            _ => issues.push(blocker_issue(
                Some(branch_name.to_owned()),
                "branch-protection-unsupported-actor",
                format!(
                    "Legacy branch protection only supports user, team, and app actors; {} is not supported",
                    actor_reference_key(actor)
                ),
            )),
        }
    }
    set.users
        .sort_by(|left, right| left.login.cmp(&right.login));
    set.teams.sort_by(|left, right| left.slug.cmp(&right.slug));
    set.apps.sort_by(|left, right| left.slug.cmp(&right.slug));
    set
}

fn protected_branch_matches(
    actual: &ProtectedBranchConfig,
    desired: &DesiredBranchProtection,
) -> bool {
    actual.protection.enabled == desired.required_pull_request_reviews
        && actual.protection.required_approvals == desired.required_approving_review_count
        && actual.protection.dismiss_stale_reviews == desired.dismiss_stale_reviews
        && actual.protection.require_code_owner_reviews == desired.require_code_owner_reviews
        && actual.protection.require_status_checks == desired.required_status_checks
        && actual.protection.strict_status_checks == desired.strict_status_checks
        && actual.protection.enforce_admins == desired.enforce_admins
        && actual.protection.required_linear_history == desired.required_linear_history
        && actual.protection.allow_force_pushes == desired.allow_force_pushes
        && actual.protection.allow_deletions == desired.allow_deletions
        && normalize_strings(actual.status_check_contexts.as_slice())
            == normalize_strings(desired.status_check_contexts.as_slice())
        && normalize_branch_status_checks(actual.status_checks.as_slice())
            == normalize_status_check_requirements(desired.status_checks.as_slice())
        && normalize_actor_refs(actual.push_restrictions.as_slice())
            == normalize_actor_set(&desired.push_restrictions)
        && normalize_actor_refs(actual.dismissal_restrictions.as_slice())
            == normalize_actor_set(&desired.dismissal_restrictions)
        && normalize_actor_refs(actual.pull_request_bypass_allowances.as_slice())
            == normalize_actor_set(&desired.pull_request_bypass_allowances)
        && actual.require_last_push_approval == desired.require_last_push_approval
        && actual.block_creations == desired.block_creations
        && normalize_optional_json(actual.required_reviewers.as_ref())
            == normalize_optional_json(desired.required_reviewers.as_ref())
        && actual.require_conversation_resolution == desired.require_conversation_resolution
        && actual.require_signed_commits == desired.require_signed_commits
        && actual.lock_branch == desired.lock_branch
        && actual.allow_fork_syncing == desired.allow_fork_syncing
}

fn normalize_actor_set(set: &ActorSet) -> Vec<String> {
    let mut normalized = set
        .users
        .iter()
        .map(|user| format!("user:{}", user.login))
        .chain(set.teams.iter().map(|team| format!("team:{}", team.slug)))
        .chain(set.apps.iter().map(|app| format!("app:{}", app.slug)))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_branch_status_checks(
    values: &[BranchStatusCheckConfig],
) -> Vec<(String, Option<i64>)> {
    let mut normalized = values
        .iter()
        .map(|check| (check.context.clone(), check.app_id))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_status_check_requirements(
    values: &[StatusCheckRequirement],
) -> Vec<(String, Option<i64>)> {
    let mut normalized = values
        .iter()
        .map(|check| (check.context.clone(), check.app_id))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_optional_json(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(|value| serde_json::to_string(value).ok())
}

fn normalize_strings(values: &[String]) -> Vec<String> {
    let mut normalized = values.to_vec();
    normalized.sort();
    normalized
}

fn branch_action_sort_key(action: &BranchProtectionPlanAction) -> (u8, String) {
    match action {
        BranchProtectionPlanAction::Upsert { branch, .. } => (0, branch.clone()),
        BranchProtectionPlanAction::Delete { branch } => (1, branch.clone()),
        BranchProtectionPlanAction::Unchanged { branch } => (2, branch.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_matches_compare_actor_sets_and_contexts() {
        let actual = ProtectedBranchConfig {
            name: "main".to_owned(),
            protection: BranchProtectionConfig {
                enabled: true,
                required_approvals: 2,
                dismiss_stale_reviews: true,
                require_code_owner_reviews: true,
                require_status_checks: true,
                strict_status_checks: true,
                enforce_admins: true,
                required_linear_history: true,
                allow_force_pushes: false,
                allow_deletions: false,
            },
            status_check_contexts: vec!["ci".to_owned(), "lint".to_owned()],
            status_checks: Vec::new(),
            push_restrictions: vec![ActorReference::Team {
                slug: "platform".to_owned(),
            }],
            dismissal_restrictions: vec![ActorReference::User {
                login: "alice".to_owned(),
            }],
            pull_request_bypass_allowances: vec![ActorReference::App {
                slug: "release-bot".to_owned(),
            }],
            require_last_push_approval: None,
            block_creations: None,
            required_reviewers: None,
            require_conversation_resolution: Some(true),
            require_signed_commits: Some(true),
            lock_branch: Some(false),
            allow_fork_syncing: Some(false),
        };
        let desired = DesiredBranchProtection {
            required_pull_request_reviews: true,
            required_approving_review_count: 2,
            dismiss_stale_reviews: true,
            require_code_owner_reviews: true,
            require_last_push_approval: None,
            required_status_checks: true,
            strict_status_checks: true,
            status_check_contexts: vec!["lint".to_owned(), "ci".to_owned()],
            status_checks: Vec::new(),
            push_restrictions: ActorSet {
                users: Vec::new(),
                teams: vec![TeamActor {
                    slug: "platform".to_owned(),
                }],
                apps: Vec::new(),
            },
            dismissal_restrictions: ActorSet {
                users: vec![UserActor {
                    login: "alice".to_owned(),
                }],
                teams: Vec::new(),
                apps: Vec::new(),
            },
            pull_request_bypass_allowances: ActorSet {
                users: Vec::new(),
                teams: Vec::new(),
                apps: vec![AppActor {
                    slug: "release-bot".to_owned(),
                }],
            },
            enforce_admins: true,
            required_linear_history: true,
            allow_force_pushes: false,
            allow_deletions: false,
            block_creations: None,
            require_conversation_resolution: Some(true),
            require_signed_commits: Some(true),
            lock_branch: Some(false),
            allow_fork_syncing: Some(false),
            required_reviewers: None,
        };

        assert!(protected_branch_matches(&actual, &desired));
    }
}
