use anyhow::{Context, Result};

use crate::reconcile::common::issue::{IssueSeverity, ReconcileIssue};
use crate::reconcile::common::issue::{has_blocker, write_outcome_issue};
use crate::reconcile::common::secrets::seal_or_block;

use crate::config::manifest::{ActionsCategory, ReferencedResourceType};
use crate::github::Client;
use crate::github::actions::{self, WriteOutcome};

use super::references::reference_kind_label;
use super::*;

/// Apply a previously computed [`ActionsPlan`]. Blocked writes (409/403/404/422)
/// are surfaced as issues rather than aborting the whole run; transport-level
/// failures still propagate as `Err`.
pub async fn apply_actions_plan(
    client: &Client,
    repo: &str,
    plan: &ActionsPlan,
) -> Result<ActionsApplyResult> {
    let mut applied = Vec::new();
    let mut issues: Vec<ReconcileIssue> = plan
        .issues
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Blocker)
        .cloned()
        .collect();

    for change in &plan.settings_changes {
        let (scope, outcome) = match change {
            ActionsSettingChange::Permissions {
                enabled,
                allowed_actions,
                sha_pinning_required,
            } => (
                "actions.settings.permissions",
                client
                    .set_actions_permissions(
                        repo,
                        &actions::ActionsPermissions {
                            enabled: *enabled,
                            allowed_actions: allowed_actions.clone(),
                            sha_pinning_required: *sha_pinning_required,
                        },
                    )
                    .await?,
            ),
            ActionsSettingChange::SelectedActions {
                github_owned_allowed,
                verified_allowed,
                patterns_allowed,
            } => (
                "actions.settings.selected_actions",
                client
                    .set_selected_actions(
                        repo,
                        &actions::SelectedActionsPolicy {
                            github_owned_allowed: *github_owned_allowed,
                            verified_allowed: *verified_allowed,
                            patterns_allowed: patterns_allowed.clone(),
                        },
                    )
                    .await?,
            ),
            ActionsSettingChange::WorkflowPermissions {
                default_workflow_permissions,
                can_approve_pull_request_reviews,
            } => (
                "actions.settings.workflow_permissions",
                client
                    .set_workflow_permissions(
                        repo,
                        &actions::WorkflowPermissions {
                            default_workflow_permissions: default_workflow_permissions.clone(),
                            can_approve_pull_request_reviews: *can_approve_pull_request_reviews,
                        },
                    )
                    .await?,
            ),
            ActionsSettingChange::ArtifactLogRetention { days } => (
                "actions.settings.artifact_log_retention",
                client.set_artifact_log_retention(repo, *days).await?,
            ),
            ActionsSettingChange::CacheRetentionLimit {
                max_cache_retention_days,
            } => (
                "actions.settings.cache_retention_limit_days",
                client
                    .set_actions_cache_retention_limit(repo, *max_cache_retention_days)
                    .await?,
            ),
            ActionsSettingChange::CacheStorageLimit { max_cache_size_gb } => (
                "actions.settings.cache_storage_limit_gb",
                client
                    .set_actions_cache_storage_limit(repo, *max_cache_size_gb)
                    .await?,
            ),
            ActionsSettingChange::ForkPrContributorApproval { approval_policy } => (
                "actions.settings.fork_pr_contributor_approval",
                client
                    .set_fork_pr_contributor_approval(repo, approval_policy)
                    .await?,
            ),
            ActionsSettingChange::PrivateForkPrWorkflows {
                run_workflows_from_fork_pull_requests,
                send_write_tokens_to_workflows,
                send_secrets_and_variables,
                require_approval_for_fork_pr_workflows,
            } => {
                // Read-modify-write: any of the three sibling booleans the
                // manifest didn't explicitly specify (`None` in the plan)
                // must be preserved from the live value, not reset to
                // `false`. Re-fetch the current state immediately before
                // writing so we merge against the freshest snapshot.
                let current = client
                    .get_private_fork_pr_workflows_checked(repo)
                    .await
                    .context(
                        "Failed to re-read private-repo fork PR workflow settings before applying",
                    )?
                    .available();
                let settings = actions::PrivateForkPrWorkflows {
                    run_workflows_from_fork_pull_requests: *run_workflows_from_fork_pull_requests,
                    send_write_tokens_to_workflows: send_write_tokens_to_workflows.unwrap_or_else(
                        || {
                            current
                                .as_ref()
                                .map(|current| current.send_write_tokens_to_workflows)
                                .unwrap_or(false)
                        },
                    ),
                    send_secrets_and_variables: send_secrets_and_variables.unwrap_or_else(|| {
                        current
                            .as_ref()
                            .map(|current| current.send_secrets_and_variables)
                            .unwrap_or(false)
                    }),
                    require_approval_for_fork_pr_workflows: require_approval_for_fork_pr_workflows
                        .unwrap_or_else(|| {
                            current
                                .as_ref()
                                .map(|current| current.require_approval_for_fork_pr_workflows)
                                .unwrap_or(false)
                        }),
                };
                (
                    "actions.settings.private_fork_workflows_enabled",
                    client
                        .set_private_fork_pr_workflows(repo, &settings)
                        .await?,
                )
            }
            ActionsSettingChange::WorkflowAccessLevel { access_level } => (
                "actions.settings.workflow_access_level",
                client.set_workflow_access_level(repo, access_level).await?,
            ),
            ActionsSettingChange::OidcSubjectClaim {
                use_default,
                include_claim_keys,
            } => (
                "actions.settings.oidc_subject_claim",
                client
                    .set_oidc_subject_claim(repo, *use_default, include_claim_keys)
                    .await?,
            ),
        };

        if let Some(issue) = write_outcome_issue(scope, outcome, &mut applied) {
            issues.push(issue);
        }
    }

    for change in &plan.workflow_state_changes {
        let scope = format!("actions.workflows.{}", change.path);
        match client.find_workflow_by_path(repo, &change.path).await? {
            Some(workflow) => {
                let outcome = if change.enabled {
                    client.enable_workflow(repo, workflow.id).await?
                } else {
                    client.disable_workflow(repo, workflow.id).await?
                };
                if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
                    issues.push(issue);
                }
            }
            None => issues.push(ReconcileIssue::blocker(
                &scope,
                "Workflow file not found in this repository",
            )),
        }
    }

    for variable in &plan.variable_upserts {
        let scope = format!("actions.variables.{}", variable.name);
        let existing = client.list_actions_variables(repo).await?;
        let outcome = if existing.iter().any(|current| current.name == variable.name) {
            client
                .update_actions_variable(repo, &variable.name, &variable.value)
                .await?
        } else {
            client
                .create_actions_variable(repo, &variable.name, &variable.value)
                .await?
        };
        if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
            issues.push(issue);
        }
    }

    for name in &plan.variable_deletions {
        let scope = format!("actions.variables.{name}");
        let outcome = client.delete_actions_variable(repo, name).await?;
        if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
            issues.push(issue);
        }
    }

    if !plan.secret_upserts.is_empty() {
        let public_key = client
            .get_actions_public_key(repo)
            .await
            .context("Failed to fetch the Actions secrets public key")?;
        for secret in &plan.secret_upserts {
            let scope = format!("actions.secrets.{}", secret.name);
            match seal_or_block(&public_key.key, &secret.name, &secret.value) {
                Ok(encrypted_value) => {
                    let outcome = client
                        .put_actions_secret(
                            repo,
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

    for name in &plan.secret_deletions {
        let scope = format!("actions.secrets.{name}");
        let outcome = client.delete_actions_secret(repo, name).await?;
        if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
            issues.push(issue);
        }
    }

    // Organization secret/variable references: only the per-repository
    // `selected` association is ever written here — never the org
    // resource's own value or visibility.
    for action in &plan.reference_actions {
        let (resource, outcome) = match action {
            OrgReferenceAction::Associate(resource) => {
                let outcome = match resource.resource_type {
                    ReferencedResourceType::OrganizationSecret => {
                        client
                            .associate_org_secret_with_repo(&resource.name, repo)
                            .await?
                    }
                    ReferencedResourceType::OrganizationVariable => {
                        client
                            .associate_org_variable_with_repo(&resource.name, repo)
                            .await?
                    }
                    _ => WriteOutcome::Blocked("reference type is observe-only".to_owned()),
                };
                (resource, outcome)
            }
            OrgReferenceAction::Disassociate(resource) => {
                let outcome = match resource.resource_type {
                    ReferencedResourceType::OrganizationSecret => {
                        client
                            .disassociate_org_secret_from_repo(&resource.name, repo)
                            .await?
                    }
                    ReferencedResourceType::OrganizationVariable => {
                        client
                            .disassociate_org_variable_from_repo(&resource.name, repo)
                            .await?
                    }
                    _ => WriteOutcome::Blocked("reference type is observe-only".to_owned()),
                };
                (resource, outcome)
            }
        };
        let scope = format!(
            "actions.references.{}.{}",
            reference_kind_label(resource.resource_type),
            resource.name
        );
        if let Some(issue) = write_outcome_issue(&scope, outcome, &mut applied) {
            issues.push(issue);
        }
    }

    Ok(ActionsApplyResult { applied, issues })
}

/// Re-collect and re-plan against `desired` to confirm convergence.
pub async fn verify_actions_category(
    client: &Client,
    repo: &str,
    desired: &ActionsCategory,
) -> Result<ActionsVerifyResult> {
    let actual = collect_actions_category(client, repo, Some(desired)).await?;
    let plan = plan_actions_category(desired, &actual);
    let compliant = !plan.has_actionable_changes() && !has_blocker(&plan.issues);
    Ok(ActionsVerifyResult { compliant, plan })
}
