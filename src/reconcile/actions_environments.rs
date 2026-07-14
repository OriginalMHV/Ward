//! Snapshot and reconciliation for `ActionsCategoryV2` and `EnvironmentsCategoryV2`.
//!
//! Follows the collect/plan/apply/verify shape used by sibling reconcile
//! modules: `collect_*` observes live GitHub state (scoped to what `desired`
//! asks about, where enumerating "everything" would otherwise be unbounded),
//! `plan_*` is a pure/sync diff against that observation, `apply_*` executes
//! the plan, and `verify_*` re-collects and re-plans to confirm convergence.
//!
//! Secret values are never logged or persisted in plaintext: GitHub never
//! returns secret values, so collected secrets are represented as
//! placeholder entries; resolved plaintext lives only in [`SecretValue`],
//! whose `Debug` impl always redacts.

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Context, Result};

use crate::config::manifest::{
    ActionsCategoryV2, ActionsSettingsConfig, ActorReference, CategoryPolicy, CoverageEntry,
    CoverageOutcome, EnvironmentConfigV2, EnvironmentDeploymentPolicyConfig,
    EnvironmentsCategoryV2, ExternalValueReference, ManagementDisposition, ManifestCategoryName,
    NamedValueConfig, ReferencedResourceConfig, ReferencedResourceType, SecretPlaceholderConfig,
    WorkflowStateConfig,
};
use crate::github::Client;
use crate::github::access::{NamedRepository, OrgScopedResourceMetadata};
use crate::github::actions::{self, WriteOutcome};
use crate::github::environments::{
    self, DeploymentBranchPolicySummary, EnvironmentReviewerInput, EnvironmentUpdate,
};

// ---------------------------------------------------------------------------
// Shared issue/severity vocabulary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Observational or non-blocking: the desired configuration could not be
    /// applied as specified, but this does not indicate a failure.
    Warning,
    /// Prevents part of the plan from being applied (unresolved secret,
    /// invalid combination, endpoint not applicable, etc.).
    Blocker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileIssue {
    /// Dotted path identifying what the issue concerns, e.g.
    /// `actions.settings.enabled` or `environments.production.secrets.TOKEN`.
    pub scope: String,
    pub severity: IssueSeverity,
    pub message: String,
}

impl ReconcileIssue {
    fn warning(scope: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            severity: IssueSeverity::Warning,
            message: message.into(),
        }
    }

    fn blocker(scope: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            severity: IssueSeverity::Blocker,
            message: message.into(),
        }
    }
}

fn has_blocker(issues: &[ReconcileIssue]) -> bool {
    issues
        .iter()
        .any(|issue| issue.severity == IssueSeverity::Blocker)
}

// ---------------------------------------------------------------------------
// Secret handling: resolution, redaction, encryption
// ---------------------------------------------------------------------------

/// A resolved secret plaintext value. `Debug` always redacts; the plaintext
/// is only ever exposed to the sealed-box encryption call at apply time.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn expose_for_encryption(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(REDACTED)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSecret {
    pub name: String,
    pub value: SecretValue,
}

/// Resolve a [`ExternalValueReference`] to a plaintext value. Returns `Err`
/// with a safe (non-sensitive) reason string on failure; the reason never
/// contains any resolved value.
fn resolve_external_value(reference: &ExternalValueReference) -> Result<SecretValue, String> {
    match reference {
        ExternalValueReference::Env { key } => std::env::var(key)
            .map(SecretValue)
            .map_err(|_| format!("environment variable `{key}` is not set")),
        ExternalValueReference::Manual { hint } => Err(match hint {
            Some(hint) => format!("value must be provided manually ({hint})"),
            None => "value must be provided manually".to_owned(),
        }),
    }
}

fn resolve_secrets(
    placeholders: &[SecretPlaceholderConfig],
    scope_prefix: &str,
    issues: &mut Vec<ReconcileIssue>,
) -> Vec<ResolvedSecret> {
    let mut resolved = Vec::new();
    for placeholder in placeholders {
        match resolve_external_value(&placeholder.value_from) {
            Ok(value) => resolved.push(ResolvedSecret {
                name: placeholder.name.clone(),
                value,
            }),
            Err(reason) => issues.push(ReconcileIssue::blocker(
                format!("{scope_prefix}.secrets.{}", placeholder.name),
                format!("Cannot resolve secret `{}`: {reason}", placeholder.name),
            )),
        }
    }
    resolved
}

/// Fetch a public key and seal `value` for it, converting encryption
/// failures into a blocked-action reason rather than propagating a hard
/// error (the plaintext is still never included in the reason).
fn seal_or_block(public_key: &str, name: &str, value: &SecretValue) -> Result<String, String> {
    actions::seal_secret_value(public_key, value.expose_for_encryption())
        .map_err(|_| format!("Failed to encrypt secret `{name}` with the target public key"))
}

fn wants_change<T: PartialEq>(desired: &Option<T>, current: &Option<T>) -> bool {
    match desired {
        Some(value) => current.as_ref() != Some(value),
        None => false,
    }
}

fn write_outcome_issue(
    scope: &str,
    outcome: WriteOutcome,
    applied: &mut Vec<String>,
) -> Option<ReconcileIssue> {
    match outcome {
        WriteOutcome::Applied(()) => {
            applied.push(scope.to_owned());
            None
        }
        WriteOutcome::Blocked(reason) => Some(ReconcileIssue::blocker(scope, reason)),
    }
}

// ---------------------------------------------------------------------------
// Actions category
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ActionsCollection {
    pub category: ActionsCategoryV2,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
    /// Resolution of organization secret/variable *references* against the
    /// target organization (existence by stable name, and — for `selected`
    /// visibility — whether this repository is associated). Populated for
    /// every name in `desired.references` plus, when `desired.policy.prune`
    /// is set, every organization secret/variable currently visible to this
    /// repository. See [`ResolvedOrgReference`].
    pub resolved_references: Vec<ResolvedOrgReference>,
}

/// The resolved state of a referenced organization secret or variable
/// against the target organization, produced during collection (network
/// access happens here, never in `plan_actions_category`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOrgReference {
    pub resource: ReferencedResourceConfig,
    /// `None` = the existence lookup was unavailable (permission denied or
    /// otherwise) and must never be treated as "absent". `Some(false)` = no
    /// resource by this stable name exists in the target organization.
    /// `Some(true)` = it exists.
    pub present: Option<bool>,
    /// Whether this repository is associated. Always `None` when
    /// `supported` is `false` (visibility is `all`/`private`, so there is
    /// nothing to associate/disassociate — the repository already has
    /// access by virtue of visibility, which ward never alters). When
    /// `supported` is `true` (visibility is `selected`), `None` means the
    /// selected-repositories list could not be resolved and association
    /// state is genuinely unknown — never assumed either way.
    pub associated: Option<bool>,
    /// `true` when visibility is `selected`, i.e. per-repository
    /// association is an actionable GitHub API concept for this resource.
    pub supported: bool,
    pub detail: Option<String>,
}

fn reference_kind_label(resource_type: ReferencedResourceType) -> &'static str {
    match resource_type {
        ReferencedResourceType::OrganizationSecret => "organization_secret",
        ReferencedResourceType::OrganizationVariable => "organization_variable",
        _ => "reference",
    }
}

/// Resolve whether `repo` is covered by a `selected`-visibility
/// organization secret/variable, given already-classified `metadata` and
/// `repositories` read outcomes. Shared by the secret/variable resolvers
/// below; mirrors the analogous pattern used for `RepositoryAccessCategoryV2`
/// references, adapted for this category's own coverage/issue vocabulary.
fn resolve_selected_repository_association(
    resource: &ReferencedResourceConfig,
    repo: &str,
    metadata_endpoint: &str,
    repositories_endpoint: &str,
    metadata: actions::ReadOutcome<Option<OrgScopedResourceMetadata>>,
    repositories: actions::ReadOutcome<Option<Vec<NamedRepository>>>,
    coverage: &mut Vec<CoverageEntry>,
) -> ResolvedOrgReference {
    let metadata = match record_read_outcome(
        coverage,
        ManifestCategoryName::Actions,
        metadata_endpoint,
        metadata,
    ) {
        Some(value) => value,
        None => {
            return ResolvedOrgReference {
                resource: resource.clone(),
                present: None,
                associated: None,
                supported: true,
                detail: Some(format!(
                    "Could not resolve organization {:?} `{}`: the lookup was unavailable (see coverage); this must not be treated as absent.",
                    resource.resource_type, resource.name
                )),
            };
        }
    };

    let Some(metadata) = metadata else {
        return ResolvedOrgReference {
            resource: resource.clone(),
            present: Some(false),
            associated: None,
            supported: true,
            detail: Some(format!(
                "Organization {:?} `{}` was not found in the target organization.",
                resource.resource_type, resource.name
            )),
        };
    };

    if metadata.visibility.as_deref() != Some("selected") {
        return ResolvedOrgReference {
            resource: resource.clone(),
            present: Some(true),
            associated: None,
            supported: false,
            detail: Some(format!(
                "{:?} `{}` visibility is {:?}; every repository already has access, so selected-repository association does not apply.",
                resource.resource_type, resource.name, metadata.visibility
            )),
        };
    }

    let repositories = match record_read_outcome(
        coverage,
        ManifestCategoryName::Actions,
        repositories_endpoint,
        repositories,
    ) {
        Some(value) => value,
        None => {
            return ResolvedOrgReference {
                resource: resource.clone(),
                present: Some(true),
                associated: None,
                supported: true,
                detail: Some(format!(
                    "Organization {:?} `{}` has `selected` visibility, but the selected-repository list could not be resolved (this endpoint requires org-admin scope); association state is unknown and must not be assumed.",
                    resource.resource_type, resource.name
                )),
            };
        }
    };

    ResolvedOrgReference {
        resource: resource.clone(),
        present: Some(true),
        associated: repositories
            .as_deref()
            .map(|repositories| repositories.iter().any(|entry| entry.name == repo)),
        supported: true,
        detail: None,
    }
}

async fn resolve_org_secret_reference(
    client: &Client,
    repo: &str,
    name: &str,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<ResolvedOrgReference> {
    let resource = ReferencedResourceConfig {
        resource_type: ReferencedResourceType::OrganizationSecret,
        name: name.to_owned(),
    };
    let metadata = client
        .get_org_secret_metadata_checked(name)
        .await
        .context("Failed to resolve organization secret metadata")?;
    let repositories = match &metadata {
        actions::ReadOutcome::Available(Some(meta))
            if meta.visibility.as_deref() == Some("selected") =>
        {
            client
                .list_org_secret_selected_repositories_checked(name)
                .await
                .context("Failed to resolve organization secret selected-repository associations")?
        }
        _ => actions::ReadOutcome::Available(None),
    };
    Ok(resolve_selected_repository_association(
        &resource,
        repo,
        "actions/organization-secrets/metadata",
        "actions/organization-secrets/repositories",
        metadata,
        repositories,
        coverage,
    ))
}

async fn resolve_org_variable_reference(
    client: &Client,
    repo: &str,
    name: &str,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<ResolvedOrgReference> {
    let resource = ReferencedResourceConfig {
        resource_type: ReferencedResourceType::OrganizationVariable,
        name: name.to_owned(),
    };
    let metadata = client
        .get_org_variable_metadata_checked(name)
        .await
        .context("Failed to resolve organization variable metadata")?;
    let repositories = match &metadata {
        actions::ReadOutcome::Available(Some(meta))
            if meta.visibility.as_deref() == Some("selected") =>
        {
            client
                .list_org_variable_selected_repositories_checked(name)
                .await
                .context(
                    "Failed to resolve organization variable selected-repository associations",
                )?
        }
        _ => actions::ReadOutcome::Available(None),
    };
    Ok(resolve_selected_repository_association(
        &resource,
        repo,
        "actions/organization-variables/metadata",
        "actions/organization-variables/repositories",
        metadata,
        repositories,
        coverage,
    ))
}

/// Record a classified [`actions::ReadOutcome`] into `coverage` when it is
/// not `Available`, returning the value on success. Used throughout
/// collection so an optional endpoint's 403/404/422/other failure becomes a
/// [`CoverageEntry`] instead of aborting the rest of the snapshot.
fn record_read_outcome<T>(
    coverage: &mut Vec<CoverageEntry>,
    category: ManifestCategoryName,
    endpoint: &str,
    outcome: actions::ReadOutcome<T>,
) -> Option<T> {
    match outcome {
        actions::ReadOutcome::Available(value) => Some(value),
        actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::NotApplicable,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::PermissionDenied,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::Unavailable,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
    }
}

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
    desired: Option<&ActionsCategoryV2>,
) -> Result<ActionsCollection> {
    let mut settings = ActionsSettingsConfig::default();
    let mut coverage = Vec::new();
    let mut issues = Vec::new();

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

        if permissions.allowed_actions.as_deref() == Some("selected") {
            if let Some(selected) = record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Actions,
                "actions/permissions/selected-actions",
                client
                    .get_selected_actions_checked(repo)
                    .await
                    .context("Failed to collect selected Actions allowlist")?,
            ) {
                settings.selected_actions = selected.patterns_allowed;
                settings.allow_github_owned_actions = Some(selected.github_owned_allowed);
                settings.allow_verified_creator_actions = Some(selected.verified_allowed);
            }
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

    let mut category = ActionsCategoryV2 {
        settings: Some(settings),
        ..ActionsCategoryV2::default()
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
        client
            .list_visible_organization_secrets_checked(repo)
            .await
            .context("Failed to collect visible organization secret references")?,
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
        client
            .list_visible_organization_variables_checked(repo)
            .await
            .context("Failed to collect visible organization variable references")?,
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
        client
            .list_repository_runners_checked(repo)
            .await
            .context("Failed to collect self-hosted runner references")?,
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
        client
            .list_dependabot_secrets_checked(repo)
            .await
            .context("Failed to collect Dependabot secret metadata")?,
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
        client
            .list_codespaces_secrets_checked(repo)
            .await
            .context("Failed to collect Codespaces secret metadata")?,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionsSettingChange {
    Permissions {
        enabled: bool,
        allowed_actions: Option<String>,
        sha_pinning_required: Option<bool>,
    },
    SelectedActions {
        github_owned_allowed: bool,
        verified_allowed: bool,
        patterns_allowed: Vec<String>,
    },
    WorkflowPermissions {
        default_workflow_permissions: String,
        can_approve_pull_request_reviews: bool,
    },
    ArtifactLogRetention {
        days: u32,
    },
    CacheRetentionLimit {
        max_cache_retention_days: u32,
    },
    CacheStorageLimit {
        max_cache_size_gb: u32,
    },
    ForkPrContributorApproval {
        approval_policy: String,
    },
    PrivateForkPrWorkflows {
        run_workflows_from_fork_pull_requests: bool,
        // `None` means the manifest doesn't specify this field: apply must
        // preserve whatever value is live rather than resetting it to
        // `false`. `Some(x)` is an explicit desired override.
        send_write_tokens_to_workflows: Option<bool>,
        send_secrets_and_variables: Option<bool>,
        require_approval_for_fork_pr_workflows: Option<bool>,
    },
    WorkflowAccessLevel {
        access_level: String,
    },
    OidcSubjectClaim {
        use_default: bool,
        include_claim_keys: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStateChange {
    pub path: String,
    pub enabled: bool,
}

/// An association change for a referenced organization secret/variable.
/// Ward never issues any write against the org resource's own value or
/// visibility here — only the per-repository `selected` association is
/// ever touched, and only under the policy gates enforced in
/// `plan_actions_category`/`apply_actions_plan` (`Associate` requires
/// `managed` + `sensitive`; `Disassociate` additionally requires `prune`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgReferenceAction {
    Associate(ReferencedResourceConfig),
    Disassociate(ReferencedResourceConfig),
}

#[derive(Debug, Clone)]
pub struct ActionsPlan {
    pub settings_changes: Vec<ActionsSettingChange>,
    pub workflow_state_changes: Vec<WorkflowStateChange>,
    pub variable_upserts: Vec<NamedValueConfig>,
    pub variable_deletions: Vec<String>,
    pub secret_upserts: Vec<ResolvedSecret>,
    pub secret_deletions: Vec<String>,
    pub reference_actions: Vec<OrgReferenceAction>,
    pub issues: Vec<ReconcileIssue>,
}

impl ActionsPlan {
    pub fn has_actionable_changes(&self) -> bool {
        !self.settings_changes.is_empty()
            || !self.workflow_state_changes.is_empty()
            || !self.variable_upserts.is_empty()
            || !self.variable_deletions.is_empty()
            || !self.secret_upserts.is_empty()
            || !self.secret_deletions.is_empty()
            || !self.reference_actions.is_empty()
    }
}

fn is_not_applicable(coverage: &[CoverageEntry], endpoint: &str) -> bool {
    coverage
        .iter()
        .any(|entry| entry.endpoint == endpoint && entry.outcome == CoverageOutcome::NotApplicable)
}

/// Diff `desired` against a prior [`collect_actions_category`] observation.
/// Pure and synchronous: any resolution requiring network access (e.g.
/// actor login/slug to id) happens at apply time.
pub fn plan_actions_category(
    desired: &ActionsCategoryV2,
    actual: &ActionsCollection,
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
        if wants_change(&wanted.enabled, &current.enabled)
            || wants_change(&wanted.allowed_actions, &current.allowed_actions)
            || wants_change(
                &wanted.requires_pinned_actions,
                &current.requires_pinned_actions,
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
                &wanted.allow_github_owned_actions,
                &current.allow_github_owned_actions,
            ) || wants_change(
                &wanted.allow_verified_creator_actions,
                &current.allow_verified_creator_actions,
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
            &wanted.default_workflow_permissions,
            &current.default_workflow_permissions,
        ) || wants_change(
            &wanted.can_approve_pull_request_reviews,
            &current.can_approve_pull_request_reviews,
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
            &wanted.cache_retention_limit_days,
            &current.cache_retention_limit_days,
        ) {
            if let Some(max_cache_retention_days) = wanted.cache_retention_limit_days {
                settings_changes.push(ActionsSettingChange::CacheRetentionLimit {
                    max_cache_retention_days,
                });
            }
        }

        if wants_change(
            &wanted.cache_storage_limit_gb,
            &current.cache_storage_limit_gb,
        ) {
            if let Some(max_cache_size_gb) = wanted.cache_storage_limit_gb {
                settings_changes
                    .push(ActionsSettingChange::CacheStorageLimit { max_cache_size_gb });
            }
        }

        if wants_change(
            &wanted.fork_pull_request_contributor_approval,
            &current.fork_pull_request_contributor_approval,
        ) {
            settings_changes.push(ActionsSettingChange::ForkPrContributorApproval {
                approval_policy: wanted
                    .fork_pull_request_contributor_approval
                    .clone()
                    .unwrap(),
            });
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
    let mut secret_upserts = resolve_secrets(&missing_secrets, "actions", &mut issues);
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

#[derive(Debug, Clone, Default)]
pub struct ActionsApplyResult {
    pub applied: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

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

#[derive(Debug, Clone)]
pub struct ActionsVerifyResult {
    pub compliant: bool,
    pub plan: ActionsPlan,
}

/// Re-collect and re-plan against `desired` to confirm convergence.
pub async fn verify_actions_category(
    client: &Client,
    repo: &str,
    desired: &ActionsCategoryV2,
) -> Result<ActionsVerifyResult> {
    let actual = collect_actions_category(client, repo, Some(desired)).await?;
    let plan = plan_actions_category(desired, &actual);
    let compliant = !plan.has_actionable_changes() && !has_blocker(&plan.issues);
    Ok(ActionsVerifyResult { compliant, plan })
}

// ---------------------------------------------------------------------------
// Environments category
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EnvironmentsCollection {
    pub category: EnvironmentsCategoryV2,
    /// Every environment name observed on the repository, regardless of
    /// whether it was in `desired` and therefore deep-collected. Needed so
    /// `plan_environments_category` can detect prune candidates that were
    /// filtered out of `category.entries` for collection efficiency.
    pub observed_names: Vec<String>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

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
    desired: Option<&EnvironmentsCategoryV2>,
) -> Result<EnvironmentsCollection> {
    let mut issues = Vec::new();
    let mut coverage = Vec::new();

    // A collected/observed snapshot must carry a sensible policy: preserve
    // the caller's `desired.policy` when reconciling against a manifest, or
    // default to `observe_sensitive()` for a bare import snapshot (never
    // silently downgrade to a plain, non-sensitive `Observe` default that
    // would misrepresent environment secrets/reviewers as low-sensitivity).
    let policy = desired
        .map(|desired| desired.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    let observed = match record_read_outcome(
        &mut coverage,
        ManifestCategoryName::Environments,
        "environments",
        client
            .list_environments_checked(repo)
            .await
            .context("Failed to list repository environments")?,
    ) {
        Some(observed) => observed,
        None => {
            return Ok(EnvironmentsCollection {
                category: EnvironmentsCategoryV2 {
                    policy,
                    ..EnvironmentsCategoryV2::default()
                },
                observed_names: Vec::new(),
                coverage,
                issues,
            });
        }
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
        if let Some(names) = &wanted_names {
            if !names.contains(env.name.as_str()) {
                continue;
            }
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
            if !branch_policies.is_empty() {
                let policy = deployment_policy
                    .get_or_insert_with(EnvironmentDeploymentPolicyConfig::default);
                for branch_policy in &branch_policies {
                    match branch_policy.policy_type.as_str() {
                        "tag" => policy.tag_patterns.push(branch_policy.name.clone()),
                        _ => policy.branch_patterns.push(branch_policy.name.clone()),
                    }
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

        entries.push(EnvironmentConfigV2 {
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
        category: EnvironmentsCategoryV2 { policy, entries },
        observed_names: observed.iter().map(|env| env.name.clone()).collect(),
        coverage,
        issues: std::mem::take(&mut issues),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvironmentSettingsChange {
    pub wait_timer_minutes: Option<u32>,
    pub prevent_self_review: Option<bool>,
    pub reviewers: Vec<ActorReference>,
    pub deployment_branch_policy: Option<DeploymentBranchPolicySummary>,
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentPlan {
    pub name: String,
    pub create: bool,
    pub settings_change: Option<EnvironmentSettingsChange>,
    pub branch_policy_creates: Vec<(String, String)>,
    pub branch_policy_deletes: Vec<u64>,
    pub protection_app_enables: Vec<String>,
    /// App slugs to disable (resolved to a numeric protection-rule id at
    /// apply time, since the collected state only carries slugs).
    pub protection_app_disables: Vec<String>,
    pub variable_upserts: Vec<NamedValueConfig>,
    pub variable_deletions: Vec<String>,
    pub secret_upserts: Vec<ResolvedSecret>,
    pub secret_deletions: Vec<String>,
}

impl EnvironmentPlan {
    pub fn has_actionable_changes(&self) -> bool {
        self.create
            || self.settings_change.is_some()
            || !self.branch_policy_creates.is_empty()
            || !self.branch_policy_deletes.is_empty()
            || !self.protection_app_enables.is_empty()
            || !self.protection_app_disables.is_empty()
            || !self.variable_upserts.is_empty()
            || !self.variable_deletions.is_empty()
            || !self.secret_upserts.is_empty()
            || !self.secret_deletions.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentsPlan {
    pub environment_plans: Vec<EnvironmentPlan>,
    pub environment_deletions: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

impl EnvironmentsPlan {
    pub fn has_actionable_changes(&self) -> bool {
        !self.environment_deletions.is_empty()
            || self
                .environment_plans
                .iter()
                .any(EnvironmentPlan::has_actionable_changes)
    }
}

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
    desired: &EnvironmentsCategoryV2,
    actual: &EnvironmentsCollection,
) -> EnvironmentsPlan {
    let mut issues = actual.issues.clone();

    if desired.policy.disposition != ManagementDisposition::Managed {
        return EnvironmentsPlan {
            environment_plans: Vec::new(),
            environment_deletions: Vec::new(),
            issues,
        };
    }

    let actual_by_name: BTreeMap<&str, &EnvironmentConfigV2> = actual
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

        let wanted_branch_policy_summary = wanted.deployment_policy.as_ref().and_then(|policy| {
            Some(DeploymentBranchPolicySummary {
                protected_branches: policy.protected_branches?,
                custom_branch_policies: policy.custom_branch_policies.unwrap_or(false),
            })
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
        plan.secret_upserts = resolve_secrets(&missing_secrets, &scope_prefix, &mut issues);
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

#[derive(Debug, Clone, Default)]
pub struct EnvironmentsApplyResult {
    pub applied: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

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

#[derive(Debug, Clone)]
pub struct EnvironmentsVerifyResult {
    pub compliant: bool,
    pub plan: EnvironmentsPlan,
}

/// Re-collect and re-plan against `desired` to confirm convergence.
pub async fn verify_environments_category(
    client: &Client,
    repo: &str,
    desired: &EnvironmentsCategoryV2,
) -> Result<EnvironmentsVerifyResult> {
    let actual = collect_environments_category(client, repo, Some(desired)).await?;
    let plan = plan_environments_category(desired, &actual);
    let compliant = !plan.has_actionable_changes() && !has_blocker(&plan.issues);
    Ok(EnvironmentsVerifyResult { compliant, plan })
}
