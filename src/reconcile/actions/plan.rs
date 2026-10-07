use std::collections::BTreeMap;

use crate::reconcile::common::issue::ReconcileIssue;
use crate::reconcile::common::secrets::resolve_secrets;
use crate::reconcile::common::secrets::{EnvLookup, process_env};

use crate::config::manifest::{
    ActionsCategory, CoverageEntry, CoverageOutcome, ManagementDisposition, ReferencedResourceType,
    SecretPlaceholderConfig,
};

use super::references::reference_kind_label;
use super::*;

fn wants_change<T: PartialEq>(desired: Option<&T>, current: Option<&T>) -> bool {
    desired.is_some_and(|value| current != Some(value))
}

fn is_not_applicable(coverage: &[CoverageEntry], endpoint: &str) -> bool {
    coverage
        .iter()
        .any(|entry| entry.endpoint == endpoint && entry.outcome == CoverageOutcome::NotApplicable)
}

/// Diff `desired` against a prior [`collect_actions_category`] observation.
/// Pure and synchronous: any resolution requiring network access (e.g.
/// actor login/slug to id) happens at apply time.
pub fn plan_actions_category(desired: &ActionsCategory, actual: &ActionsCollection) -> ActionsPlan {
    plan_actions_category_with_env(desired, actual, &process_env)
}

/// As [`plan_actions_category`], resolving secret values through `env`.
pub fn plan_actions_category_with_env(
    desired: &ActionsCategory,
    actual: &ActionsCollection,
    env: EnvLookup<'_>,
) -> ActionsPlan {
    let mut issues = actual.issues.clone();

    if desired.policy.disposition != ManagementDisposition::Managed {
        return ActionsPlan {
            settings_changes: Vec::new(),
            workflow_state_changes: Vec::new(),
            variable_upserts: Vec::new(),
            variable_deletions: Vec::new(),
            secret_upserts: Vec::new(),
            secret_deletions: Vec::new(),
            reference_actions: Vec::new(),
            issues,
        };
    }

    let current = actual.category.settings.clone().unwrap_or_default();
    let mut settings_changes = Vec::new();

    if let Some(wanted) = &desired.settings {
        // Repository Actions permissions (enabled/allowed_actions/sha pinning).
        if wants_change(wanted.enabled.as_ref(), current.enabled.as_ref())
            || wants_change(
                wanted.allowed_actions.as_ref(),
                current.allowed_actions.as_ref(),
            )
            || wants_change(
                wanted.requires_pinned_actions.as_ref(),
                current.requires_pinned_actions.as_ref(),
            )
        {
            match wanted.enabled {
                Some(enabled) => settings_changes.push(ActionsSettingChange::Permissions {
                    enabled,
                    allowed_actions: wanted
                        .allowed_actions
                        .clone()
                        .or_else(|| current.allowed_actions.clone()),
                    sha_pinning_required: wanted
                        .requires_pinned_actions
                        .or(current.requires_pinned_actions),
                }),
                None => issues.push(ReconcileIssue::blocker(
                    "actions.settings.enabled",
                    "`enabled` must be specified to manage Actions permissions",
                )),
            }
        }

        // Selected-actions allowlist only applies when allowed_actions == "selected".
        let effective_allowed_actions = wanted
            .allowed_actions
            .as_deref()
            .or(current.allowed_actions.as_deref());
        if effective_allowed_actions == Some("selected")
            && (wants_change(
                wanted.allow_github_owned_actions.as_ref(),
                current.allow_github_owned_actions.as_ref(),
            ) || wants_change(
                wanted.allow_verified_creator_actions.as_ref(),
                current.allow_verified_creator_actions.as_ref(),
            ) || (!wanted.selected_actions.is_empty()
                && wanted.selected_actions != current.selected_actions))
        {
            settings_changes.push(ActionsSettingChange::SelectedActions {
                github_owned_allowed: wanted
                    .allow_github_owned_actions
                    .unwrap_or(current.allow_github_owned_actions.unwrap_or(false)),
                verified_allowed: wanted
                    .allow_verified_creator_actions
                    .unwrap_or(current.allow_verified_creator_actions.unwrap_or(false)),
                patterns_allowed: if wanted.selected_actions.is_empty() {
                    current.selected_actions.clone()
                } else {
                    wanted.selected_actions.clone()
                },
            });
        }

        // Workflow (default GITHUB_TOKEN) permissions.
        if wants_change(
            wanted.default_workflow_permissions.as_ref(),
            current.default_workflow_permissions.as_ref(),
        ) || wants_change(
            wanted.can_approve_pull_request_reviews.as_ref(),
            current.can_approve_pull_request_reviews.as_ref(),
        ) {
            let default_permissions = wanted
                .default_workflow_permissions
                .clone()
                .or_else(|| current.default_workflow_permissions.clone());
            let can_approve = wanted
                .can_approve_pull_request_reviews
                .or(current.can_approve_pull_request_reviews);
            match (default_permissions, can_approve) {
                (Some(default_workflow_permissions), Some(can_approve_pull_request_reviews)) => {
                    settings_changes.push(ActionsSettingChange::WorkflowPermissions {
                        default_workflow_permissions,
                        can_approve_pull_request_reviews,
                    });
                }
                _ => issues.push(ReconcileIssue::blocker(
                    "actions.settings.default_workflow_permissions",
                    "Both `default_workflow_permissions` and `can_approve_pull_request_reviews` must be resolvable to manage workflow permissions",
                )),
            }
        }

        // Artifact/log retention: GitHub only exposes a single combined value.
        match (wanted.artifact_retention_days, wanted.log_retention_days) {
            (Some(artifact_days), Some(log_days)) if artifact_days != log_days => {
                issues.push(ReconcileIssue::blocker(
                    "actions.settings.artifact_retention_days",
                    format!(
                        "`artifact_retention_days` ({artifact_days}) and `log_retention_days` ({log_days}) conflict: GitHub exposes only a single combined retention setting"
                    ),
                ));
            }
            (Some(days), _) | (_, Some(days)) => {
                if current.artifact_retention_days != Some(days) {
                    settings_changes.push(ActionsSettingChange::ArtifactLogRetention { days });
                }
            }
            (None, None) => {}
        }

        if wants_change(
            wanted.cache_retention_limit_days.as_ref(),
            current.cache_retention_limit_days.as_ref(),
        ) && let Some(max_cache_retention_days) = wanted.cache_retention_limit_days
        {
            settings_changes.push(ActionsSettingChange::CacheRetentionLimit {
                max_cache_retention_days,
            });
        }

        if wants_change(
            wanted.cache_storage_limit_gb.as_ref(),
            current.cache_storage_limit_gb.as_ref(),
        ) && let Some(max_cache_size_gb) = wanted.cache_storage_limit_gb
        {
            settings_changes.push(ActionsSettingChange::CacheStorageLimit { max_cache_size_gb });
        }

        if wants_change(
            wanted.fork_pull_request_contributor_approval.as_ref(),
            current.fork_pull_request_contributor_approval.as_ref(),
        ) && let Some(approval_policy) = wanted.fork_pull_request_contributor_approval.clone()
        {
            settings_changes
                .push(ActionsSettingChange::ForkPrContributorApproval { approval_policy });
        }

        // Private/internal-repo-only fork PR workflow policy. Both manifest
        // fields map onto the same GitHub boolean; if they disagree, that's
        // an unresolvable conflict rather than a silent pick. The three
        // sibling booleans (write tokens, secrets/variables, approval) are
        // optional overrides: when the manifest doesn't specify them,
        // `None` is carried through so apply preserves the live value
        // instead of resetting it to `false`.
        if wanted.private_fork_workflows_enabled.is_some()
            || wanted.fork_pull_request_workflows_enabled.is_some()
            || wanted.send_write_tokens_to_workflows.is_some()
            || wanted.send_secrets_and_variables.is_some()
            || wanted.require_approval_for_fork_pr_workflows.is_some()
        {
            if is_not_applicable(
                &actual.coverage,
                "actions/permissions/fork-pr-workflows-private-repos",
            ) {
                issues.push(ReconcileIssue::warning(
                    "actions.settings.private_fork_workflows_enabled",
                    "This repository is public; fork PR workflow policy only applies to private/internal repositories",
                ));
            } else {
                match (wanted.private_fork_workflows_enabled, wanted.fork_pull_request_workflows_enabled) {
                    (Some(a), Some(b)) if a != b => issues.push(ReconcileIssue::blocker(
                        "actions.settings.private_fork_workflows_enabled",
                        "`private_fork_workflows_enabled` and `fork_pull_request_workflows_enabled` conflict: both map to the same GitHub setting",
                    )),
                    (a, b) => {
                        let wanted_value = a.or(b);
                        let run_changes = wanted_value
                            .is_some_and(|value| current.private_fork_workflows_enabled != Some(value));
                        let write_tokens_changes = wanted.send_write_tokens_to_workflows.is_some_and(
                            |value| current.send_write_tokens_to_workflows != Some(value),
                        );
                        let secrets_vars_changes = wanted.send_secrets_and_variables.is_some_and(
                            |value| current.send_secrets_and_variables != Some(value),
                        );
                        let approval_changes = wanted
                            .require_approval_for_fork_pr_workflows
                            .is_some_and(|value| {
                                current.require_approval_for_fork_pr_workflows != Some(value)
                            });
                        if run_changes
                            || write_tokens_changes
                            || secrets_vars_changes
                            || approval_changes
                        {
                            settings_changes.push(ActionsSettingChange::PrivateForkPrWorkflows {
                                run_workflows_from_fork_pull_requests: wanted_value
                                    .unwrap_or_else(|| {
                                        current.private_fork_workflows_enabled.unwrap_or(false)
                                    }),
                                send_write_tokens_to_workflows: wanted.send_write_tokens_to_workflows,
                                send_secrets_and_variables: wanted.send_secrets_and_variables,
                                require_approval_for_fork_pr_workflows: wanted
                                    .require_approval_for_fork_pr_workflows,
                            });
                        }
                    }
                }
            }
        }

        if let Some(access_level) = &wanted.workflow_access_level {
            if is_not_applicable(&actual.coverage, "actions/permissions/access") {
                issues.push(ReconcileIssue::warning(
                    "actions.settings.workflow_access_level",
                    "Workflow access level only applies to private repositories",
                ));
            } else if current.workflow_access_level.as_deref() != Some(access_level.as_str()) {
                settings_changes.push(ActionsSettingChange::WorkflowAccessLevel {
                    access_level: access_level.clone(),
                });
            }
        }

        if wanted.oidc_subject_claim_template.is_some() {
            issues.push(ReconcileIssue::warning(
                "actions.settings.oidc_subject_claim_template",
                "GitHub does not expose a writable custom OIDC subject claim template; this field is observational only",
            ));
        }
        if !wanted.oidc_subject_claim_include_keys.is_empty()
            && wanted.oidc_subject_claim_include_keys != current.oidc_subject_claim_include_keys
        {
            settings_changes.push(ActionsSettingChange::OidcSubjectClaim {
                use_default: false,
                include_claim_keys: wanted.oidc_subject_claim_include_keys.clone(),
            });
        }
    }

    // Workflow enable/disable (idempotent: only included when it would change).
    let mut workflow_state_changes = Vec::new();
    let observed_workflow_state: BTreeMap<&str, bool> = actual
        .category
        .workflows
        .iter()
        .filter_map(|workflow| Some((workflow.path.as_str(), workflow.enabled?)))
        .collect();
    for wanted in &desired.workflows {
        if let Some(enabled) = wanted.enabled {
            match observed_workflow_state.get(wanted.path.as_str()) {
                Some(&current_enabled) if current_enabled == enabled => {}
                Some(_) => workflow_state_changes.push(WorkflowStateChange {
                    path: wanted.path.clone(),
                    enabled,
                }),
                None => {
                    // Missing from the observation means collect() couldn't find it
                    // (already recorded as a blocker issue during collect).
                }
            }
        }
    }

    // Actions variables: full diff by name/value, fully idempotent.
    let current_variables: BTreeMap<&str, &str> = actual
        .category
        .variables
        .iter()
        .map(|variable| (variable.name.as_str(), variable.value.as_str()))
        .collect();
    let mut variable_upserts = Vec::new();
    let mut desired_variable_names = std::collections::BTreeSet::new();
    for wanted in &desired.variables {
        desired_variable_names.insert(wanted.name.as_str());
        if current_variables.get(wanted.name.as_str()) != Some(&wanted.value.as_str()) {
            variable_upserts.push(wanted.clone());
        }
    }
    let mut variable_deletions = Vec::new();
    if desired.policy.prune {
        for name in current_variables.keys() {
            if !desired_variable_names.contains(name) {
                variable_deletions.push((*name).to_owned());
            }
        }
    }

    // Actions secrets: idempotent by name. GitHub never returns secret
    // values, so a secret whose name already exists remotely is treated as
    // present and left untouched — only names absent from the last
    // observation are resolved/upserted. This also makes `verify_*` converge
    // once a secret exists, instead of perpetually re-planning an unresolvable
    // external value that the target already has. Deletions are pruned by
    // name when `prune` is set, independent of this idempotence rule.
    let current_secret_names: std::collections::BTreeSet<&str> = actual
        .category
        .secrets
        .iter()
        .map(|secret| secret.name.as_str())
        .collect();
    let missing_secrets: Vec<SecretPlaceholderConfig> = desired
        .secrets
        .iter()
        .filter(|secret| !current_secret_names.contains(secret.name.as_str()))
        .cloned()
        .collect();
    let mut secret_upserts = resolve_secrets(&missing_secrets, "actions", &mut issues, env);
    secret_upserts.retain(|secret| !secret.name.is_empty());
    let mut secret_deletions = Vec::new();
    if desired.policy.prune {
        let desired_secret_names: std::collections::BTreeSet<&str> = desired
            .secrets
            .iter()
            .map(|secret| secret.name.as_str())
            .collect();
        for secret in &actual.category.secrets {
            if !desired_secret_names.contains(secret.name.as_str()) {
                secret_deletions.push(secret.name.clone());
            }
        }
    }

    // Dependabot/Codespaces secrets: the manifest schema carries these as
    // placeholders (name + external value source) for future round-trip
    // support, but this category does not yet implement the write path for
    // either family (separate public-key/PUT/DELETE endpoints per family).
    // Surface a warning rather than silently dropping desired input so
    // drift isn't hidden.
    if !desired.dependabot_secrets.is_empty() {
        issues.push(ReconcileIssue::warning(
            "actions.dependabot_secrets",
            format!(
                "Dependabot secret management is observational-only in this version; {} desired name(s) were not applied: {}",
                desired.dependabot_secrets.len(),
                desired
                    .dependabot_secrets
                    .iter()
                    .map(|secret| secret.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    if !desired.codespaces_secrets.is_empty() {
        issues.push(ReconcileIssue::warning(
            "actions.codespaces_secrets",
            format!(
                "Codespaces secret management is observational-only in this version; {} desired name(s) were not applied: {}",
                desired.codespaces_secrets.len(),
                desired
                    .codespaces_secrets
                    .iter()
                    .map(|secret| secret.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    // Organization secret/variable references: never alter the org
    // resource's value or visibility — only ever propose a per-repository
    // `selected` association/disassociation, and only when this category is
    // explicitly `managed` (guaranteed by this point) *and* `sensitive`.
    // Permission failures while resolving are surfaced as blockers, never
    // treated as "not associated" or silently satisfied.
    let mut reference_actions = Vec::new();
    let desired_secret_names: std::collections::BTreeSet<&str> = desired
        .references
        .iter()
        .filter(|reference| reference.resource_type == ReferencedResourceType::OrganizationSecret)
        .map(|reference| reference.name.as_str())
        .collect();
    let desired_variable_names: std::collections::BTreeSet<&str> = desired
        .references
        .iter()
        .filter(|reference| reference.resource_type == ReferencedResourceType::OrganizationVariable)
        .map(|reference| reference.name.as_str())
        .collect();

    for resolved in &actual.resolved_references {
        let scope = format!(
            "actions.references.{}.{}",
            reference_kind_label(resolved.resource.resource_type),
            resolved.resource.name
        );
        let is_desired = match resolved.resource.resource_type {
            ReferencedResourceType::OrganizationSecret => {
                desired_secret_names.contains(resolved.resource.name.as_str())
            }
            ReferencedResourceType::OrganizationVariable => {
                desired_variable_names.contains(resolved.resource.name.as_str())
            }
            _ => false,
        };

        if is_desired {
            match resolved.present {
                Some(false) => issues.push(ReconcileIssue::blocker(
                    &scope,
                    format!(
                        "Referenced {:?} `{}` does not exist in the target organization.",
                        resolved.resource.resource_type, resolved.resource.name
                    ),
                )),
                None => issues.push(ReconcileIssue::blocker(
                    &scope,
                    resolved.detail.clone().unwrap_or_else(|| {
                        format!(
                            "Could not resolve referenced {:?} `{}`.",
                            resolved.resource.resource_type, resolved.resource.name
                        )
                    }),
                )),
                Some(true) if !resolved.supported => {
                    // `all`/`private` visibility: every repository already
                    // has access; nothing to associate.
                }
                Some(true) if matches!(resolved.associated, Some(false)) => {
                    reference_actions
                        .push(OrgReferenceAction::Associate(resolved.resource.clone()));
                }
                Some(true) if resolved.associated.is_none() => issues
                    .push(ReconcileIssue::blocker(
                    &scope,
                    resolved.detail.clone().unwrap_or_else(|| {
                        format!(
                            "Could not determine selected-repository association for {:?} `{}`.",
                            resolved.resource.resource_type, resolved.resource.name
                        )
                    }),
                )),
                _ => {}
            }
        } else if desired.policy.prune {
            // Currently visible to this repository but no longer desired.
            // Only actionable when visibility is `selected` (a real,
            // reversible association); `all`/`private` visibility cannot
            // be revoked per-repository without altering the org
            // resource's visibility, which ward never does.
            match (resolved.supported, resolved.associated) {
                (true, Some(true)) => reference_actions.push(OrgReferenceAction::Disassociate(
                    resolved.resource.clone(),
                )),
                (true, None) => issues.push(ReconcileIssue::blocker(
                    &scope,
                    resolved.detail.clone().unwrap_or_else(|| {
                        format!(
                            "Could not determine selected-repository association for {:?} `{}`; cannot safely prune.",
                            resolved.resource.resource_type, resolved.resource.name
                        )
                    }),
                )),
                _ => {}
            }
        }
    }

    if !reference_actions.is_empty() && !desired.policy.sensitive {
        issues.push(ReconcileIssue::blocker(
            "actions.references",
            "Organization secret/variable association changes require `policy.sensitive: true`.",
        ));
        reference_actions.clear();
    }

    if reference_actions
        .iter()
        .any(|action| matches!(action, OrgReferenceAction::Disassociate(_)))
        && !desired.policy.prune
    {
        // Unreachable in practice (disassociation is only ever proposed
        // under `prune` above), kept as a defense-in-depth invariant so a
        // future refactor can't silently disassociate outside prune.
        reference_actions.retain(|action| !matches!(action, OrgReferenceAction::Disassociate(_)));
    }

    ActionsPlan {
        settings_changes,
        workflow_state_changes,
        variable_upserts,
        variable_deletions,
        secret_upserts,
        secret_deletions,
        reference_actions,
        issues,
    }
}
