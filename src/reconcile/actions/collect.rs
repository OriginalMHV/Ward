use anyhow::{Context, Result};

use crate::reconcile::common::coverage::record_read_outcome;
use crate::reconcile::common::issue::ReconcileIssue;
use crate::reconcile::relax_unrequested;

use crate::config::manifest::{
    ActionsCategory, ActionsSettingsConfig, CoverageEntry, CoverageOutcome, ExternalValueReference,
    ManifestCategoryName, NamedValueConfig, ReferencedResourceConfig, ReferencedResourceType,
    SecretPlaceholderConfig, WorkflowStateConfig,
};
use crate::github::Client;

use super::references::{resolve_org_secret_reference, resolve_org_variable_reference};
use super::*;

/// Collect the observable Actions configuration for `repo`. When `desired`
/// is provided, workflow enable/disable state is only resolved for the
/// workflow paths it references (enumerating every workflow otherwise would
/// be unbounded and is not needed for planning). When `desired` is `None`
/// (a source-import snapshot), every workflow's enabled state is collected,
/// since there is no desired set to narrow the read to.
///
/// Every optional sub-endpoint is read through a `_checked` client method:
/// a 403 (no permission), 404 (not supported for this repository/plan), or
/// 422 (endpoint valid in shape but not applicable, e.g. fork PR contributor
/// approval on a private repository) is recorded as a [`CoverageEntry`]
/// rather than aborting the rest of collection.
pub async fn collect_actions_category(
    client: &Client,
    repo: &str,
    desired: Option<&ActionsCategory>,
) -> Result<ActionsCollection> {
    let mut settings = ActionsSettingsConfig::default();
    let mut coverage = Vec::new();
    let mut issues = Vec::new();

    // A source import (`desired` is `None`) reads everything. A plan reads
    // optional endpoints too, but only counts a failure when the manifest manages them.
    let requests = |resource: Option<ReferencedResourceType>, has_entries: bool| {
        desired.is_none_or(|desired| {
            desired.policy.prune
                || has_entries
                || resource.is_some_and(|resource| {
                    desired
                        .references
                        .iter()
                        .any(|reference| reference.resource_type == resource)
                })
        })
    };
    let wants_org_secrets = requests(Some(ReferencedResourceType::OrganizationSecret), false);
    let wants_org_variables = requests(Some(ReferencedResourceType::OrganizationVariable), false);
    let wants_runners = requests(Some(ReferencedResourceType::Runner), false);
    let wants_dependabot_secrets = requests(
        None,
        desired.is_some_and(|d| !d.dependabot_secrets.is_empty()),
    );
    let wants_codespaces_secrets = requests(
        None,
        desired.is_some_and(|d| !d.codespaces_secrets.is_empty()),
    );

    if let Some(permissions) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/permissions",
        client
            .get_actions_permissions_checked(repo)
            .await
            .context("Failed to collect Actions permissions")?,
    ) {
        settings.enabled = Some(permissions.enabled);
        settings.allowed_actions = permissions.allowed_actions.clone();
        settings.requires_pinned_actions = permissions.sha_pinning_required;

        if permissions.allowed_actions.as_deref() == Some("selected")
            && let Some(selected) = record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Actions,
                "actions/permissions/selected-actions",
                client
                    .get_selected_actions_checked(repo)
                    .await
                    .context("Failed to collect selected Actions allowlist")?,
            )
        {
            settings.selected_actions = selected.patterns_allowed;
            settings.allow_github_owned_actions = Some(selected.github_owned_allowed);
            settings.allow_verified_creator_actions = Some(selected.verified_allowed);
        }
    }

    if let Some(workflow_permissions) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/permissions/workflow",
        client
            .get_workflow_permissions_checked(repo)
            .await
            .context("Failed to collect workflow permissions")?,
    ) {
        settings.default_workflow_permissions =
            Some(workflow_permissions.default_workflow_permissions);
        settings.can_approve_pull_request_reviews =
            Some(workflow_permissions.can_approve_pull_request_reviews);
    }

    if let Some(retention) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/permissions/artifact-and-log-retention",
        client
            .get_artifact_log_retention_checked(repo)
            .await
            .context("Failed to collect artifact/log retention")?,
    ) {
        settings.artifact_retention_days = Some(retention.days);
        settings.log_retention_days = Some(retention.days);
    }

    if let Some(cache_retention) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/cache/retention-limit",
        client
            .get_actions_cache_retention_limit_checked(repo)
            .await
            .context("Failed to collect Actions cache retention limit")?,
    ) {
        settings.cache_retention_limit_days = Some(cache_retention.max_cache_retention_days);
    }

    if let Some(cache_storage) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/cache/storage-limit",
        client
            .get_actions_cache_storage_limit_checked(repo)
            .await
            .context("Failed to collect Actions cache storage limit")?,
    ) {
        settings.cache_storage_limit_gb = Some(cache_storage.max_cache_size_gb);
    }

    if let Some(policy) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/permissions/fork-pr-contributor-approval",
        client
            .get_fork_pr_contributor_approval_checked(repo)
            .await
            .context("Failed to collect fork PR contributor approval policy")?,
    ) {
        settings.fork_pull_request_contributor_approval = Some(policy.approval_policy);
    }

    if let Some(fork_settings) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/permissions/fork-pr-workflows-private-repos",
        client
            .get_private_fork_pr_workflows_checked(repo)
            .await
            .context("Failed to collect private-repo fork PR workflow settings")?,
    ) {
        settings.private_fork_workflows_enabled =
            Some(fork_settings.run_workflows_from_fork_pull_requests);
        settings.fork_pull_request_workflows_enabled =
            Some(fork_settings.run_workflows_from_fork_pull_requests);
        settings.send_write_tokens_to_workflows =
            Some(fork_settings.send_write_tokens_to_workflows);
        settings.send_secrets_and_variables = Some(fork_settings.send_secrets_and_variables);
        settings.require_approval_for_fork_pr_workflows =
            Some(fork_settings.require_approval_for_fork_pr_workflows);
    }

    if let Some(access) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/permissions/access",
        client
            .get_workflow_access_level_checked(repo)
            .await
            .context("Failed to collect workflow access level")?,
    ) {
        settings.workflow_access_level = Some(access.access_level);
    }

    if let Some(oidc) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/oidc/customization/sub",
        client
            .get_oidc_subject_claim_checked(repo)
            .await
            .context("Failed to collect OIDC subject claim customization")?,
    ) {
        settings.oidc_subject_claim_include_keys = oidc.include_claim_keys;
        if let Some(prefix) = oidc.sub_claim_prefix {
            coverage.push(CoverageEntry {
                category: ManifestCategoryName::Actions,
                endpoint: "actions/oidc/customization/sub".to_owned(),
                outcome: CoverageOutcome::Collected,
                reason: Some(format!(
                    "Observed computed sub_claim_prefix `{prefix}`; GitHub does not expose a writable custom subject template"
                )),
                required_permission: None,
            });
        }
    }

    let mut category = ActionsCategory {
        settings: Some(settings),
        ..ActionsCategory::default()
    };

    match desired {
        Some(desired) if !desired.workflows.is_empty() => {
            if let Some(workflows) = record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Actions,
                "actions/workflows",
                client
                    .list_workflows_checked(repo)
                    .await
                    .context("Failed to list workflows")?,
            ) {
                for wanted in &desired.workflows {
                    match workflows
                        .iter()
                        .find(|workflow| workflow.path == wanted.path)
                    {
                        Some(found) => category.workflows.push(WorkflowStateConfig {
                            path: found.path.clone(),
                            enabled: Some(found.state == "active"),
                        }),
                        None => issues.push(ReconcileIssue::blocker(
                            format!("actions.workflows.{}", wanted.path),
                            "Workflow file not found in this repository",
                        )),
                    }
                }
            }
        }
        Some(_) => {
            // `desired.workflows` is empty: nothing to resolve.
        }
        None => {
            // Source-import snapshot: capture every workflow's enabled state,
            // not just ones named by a desired configuration.
            if let Some(workflows) = record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Actions,
                "actions/workflows",
                client
                    .list_workflows_checked(repo)
                    .await
                    .context("Failed to list workflows")?,
            ) {
                category.workflows = workflows
                    .into_iter()
                    .map(|workflow| WorkflowStateConfig {
                        path: workflow.path,
                        enabled: Some(workflow.state == "active"),
                    })
                    .collect();
            }
        }
    }

    if let Some(variables) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/variables",
        client
            .list_actions_variables_checked(repo)
            .await
            .context("Failed to collect Actions variables")?,
    ) {
        category.variables = variables
            .into_iter()
            .map(|variable| NamedValueConfig {
                name: variable.name,
                value: variable.value,
            })
            .collect();
    }

    if let Some(secrets) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/secrets",
        client
            .list_actions_secrets_checked(repo)
            .await
            .context("Failed to collect Actions secret metadata")?,
    ) {
        category.secrets = secrets
            .into_iter()
            .map(|secret| SecretPlaceholderConfig {
                name: secret.name,
                value_from: ExternalValueReference::Manual {
                    hint: Some("Existing secret; GitHub never returns secret values".to_owned()),
                },
            })
            .collect();
    }

    if let Some(org_secrets) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/organization-secrets",
        relax_unrequested(
            client
                .list_visible_organization_secrets_checked(repo)
                .await
                .context("Failed to collect visible organization secret references")?,
            wants_org_secrets,
        ),
    ) {
        category
            .references
            .extend(
                org_secrets
                    .into_iter()
                    .map(|secret| ReferencedResourceConfig {
                        resource_type: ReferencedResourceType::OrganizationSecret,
                        name: secret.name,
                    }),
            );
    }

    if let Some(org_variables) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/organization-variables",
        relax_unrequested(
            client
                .list_visible_organization_variables_checked(repo)
                .await
                .context("Failed to collect visible organization variable references")?,
            wants_org_variables,
        ),
    ) {
        category
            .references
            .extend(
                org_variables
                    .into_iter()
                    .map(|variable| ReferencedResourceConfig {
                        resource_type: ReferencedResourceType::OrganizationVariable,
                        name: variable.name,
                    }),
            );
    }

    // Self-hosted runners: read-only diagnostic references only. Ward never
    // registers, re-registers, or deletes runners. GitHub's runner ids and
    // runner_group_ids are internal source identifiers and must never be
    // persisted; only a stable, human-readable compact name (runner name +
    // status + sorted labels) is stored.
    if let Some(runners) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "actions/runners",
        relax_unrequested(
            client
                .list_repository_runners_checked(repo)
                .await
                .context("Failed to collect self-hosted runner references")?,
            wants_runners,
        ),
    ) {
        category
            .references
            .extend(runners.into_iter().map(|runner| {
                let mut labels: Vec<&str> = runner
                    .labels
                    .iter()
                    .map(|label| label.name.as_str())
                    .collect();
                labels.sort_unstable();
                ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::Runner,
                    name: format!(
                        "{} [status={}, labels={}]",
                        runner.name,
                        runner.status,
                        labels.join(",")
                    ),
                }
            }));
    }

    // Runner groups have no documented repository-scoped endpoint: GitHub
    // only exposes `GET /orgs/{org}/actions/runner-groups` (optionally
    // filtered with `visible_to_repository`), which requires organization-
    // admin scope rather than the repo-scoped credentials this category
    // otherwise relies on. Record this limitation explicitly rather than
    // silently omitting runner-group coverage.
    coverage.push(CoverageEntry {
        category: ManifestCategoryName::Actions,
        endpoint: "actions/runner-groups".to_owned(),
        outcome: CoverageOutcome::Unsupported,
        reason: Some(
            "GitHub does not expose a repository-scoped endpoint for runner group visibility; only the organization-scoped `GET /orgs/{org}/actions/runner-groups` endpoint (with `visible_to_repository`) supports this, which requires org-admin scope beyond this repo-focused client".to_owned(),
        ),
        required_permission: None,
    });

    // Dependabot/Codespaces secret *names* (never values) are preserved as
    // manifest placeholders — one `SecretPlaceholderConfig` per observed
    // secret, mirroring the Actions-secret collection pattern above — plus
    // one `CoverageEntry` per secret so a snapshot doesn't silently collapse
    // them into a count.
    if let Some(dependabot_secrets) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "dependabot/secrets",
        relax_unrequested(
            client
                .list_dependabot_secrets_checked(repo)
                .await
                .context("Failed to collect Dependabot secret metadata")?,
            wants_dependabot_secrets,
        ),
    ) {
        category.dependabot_secrets = dependabot_secrets
            .iter()
            .map(|secret| SecretPlaceholderConfig {
                name: secret.name.clone(),
                value_from: ExternalValueReference::Manual {
                    hint: Some(
                        "Existing Dependabot secret; GitHub never returns secret values".to_owned(),
                    ),
                },
            })
            .collect();
        for secret in &dependabot_secrets {
            coverage.push(CoverageEntry {
                category: ManifestCategoryName::Actions,
                endpoint: format!("dependabot/secrets/{}", secret.name),
                outcome: CoverageOutcome::Collected,
                reason: Some(
                    "Dependabot secret name observed and preserved as a manifest placeholder; GitHub never returns secret values".to_owned(),
                ),
                required_permission: None,
            });
        }
    }

    if let Some(codespaces_secrets) = record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Actions,
        "codespaces/secrets",
        relax_unrequested(
            client
                .list_codespaces_secrets_checked(repo)
                .await
                .context("Failed to collect Codespaces secret metadata")?,
            wants_codespaces_secrets,
        ),
    ) {
        category.codespaces_secrets = codespaces_secrets
            .iter()
            .map(|secret| SecretPlaceholderConfig {
                name: secret.name.clone(),
                value_from: ExternalValueReference::Manual {
                    hint: Some(
                        "Existing Codespaces secret; GitHub never returns secret values".to_owned(),
                    ),
                },
            })
            .collect();
        for secret in &codespaces_secrets {
            coverage.push(CoverageEntry {
                category: ManifestCategoryName::Actions,
                endpoint: format!("codespaces/secrets/{}", secret.name),
                outcome: CoverageOutcome::Collected,
                reason: Some(
                    "Codespaces secret name observed and preserved as a manifest placeholder; GitHub never returns secret values".to_owned(),
                ),
                required_permission: None,
            });
        }
    }

    // Organization secret/variable references: resolve by stable name in
    // the target organization, and — for `selected` visibility — whether
    // this repository is currently associated. Resolved for every name in
    // `desired.references` (so planning can propose a sensitive association
    // action) plus, when pruning is requested, every organization
    // secret/variable currently visible to this repository (so planning can
    // consider disassociating names no longer desired). Never resolved
    // speculatively beyond that — this mirrors the "narrow to what desired
    // asks about" approach used for workflow enumeration above.
    //
    // Kept as two separate name sets (rather than keyed by
    // `ReferencedResourceType`) since that manifest type does not derive
    // `Ord`, and this category owns neither the manifest nor its derives.
    let mut secret_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut variable_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(desired) = desired {
        for reference in &desired.references {
            match reference.resource_type {
                ReferencedResourceType::OrganizationSecret => {
                    secret_names.insert(reference.name.clone());
                }
                ReferencedResourceType::OrganizationVariable => {
                    variable_names.insert(reference.name.clone());
                }
                _ => {}
            }
        }
        if desired.policy.prune {
            for reference in &category.references {
                match reference.resource_type {
                    ReferencedResourceType::OrganizationSecret => {
                        secret_names.insert(reference.name.clone());
                    }
                    ReferencedResourceType::OrganizationVariable => {
                        variable_names.insert(reference.name.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    let mut resolved_references = Vec::new();
    for name in &secret_names {
        resolved_references
            .push(resolve_org_secret_reference(client, repo, name, &mut coverage).await?);
    }
    for name in &variable_names {
        resolved_references
            .push(resolve_org_variable_reference(client, repo, name, &mut coverage).await?);
    }

    Ok(ActionsCollection {
        category,
        coverage,
        issues,
        resolved_references,
    })
}
