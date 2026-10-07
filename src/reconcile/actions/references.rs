use anyhow::{Context, Result};

use crate::reconcile::common::coverage::record_read_outcome;

use crate::config::manifest::{
    CoverageEntry, ManifestCategoryName, ReferencedResourceConfig, ReferencedResourceType,
};
use crate::github::Client;
use crate::github::access::{NamedRepository, OrgScopedResourceMetadata};
use crate::github::actions::{self};

use super::*;

pub(super) fn reference_kind_label(resource_type: ReferencedResourceType) -> &'static str {
    match resource_type {
        ReferencedResourceType::OrganizationSecret => "organization_secret",
        ReferencedResourceType::OrganizationVariable => "organization_variable",
        _ => "reference",
    }
}

/// Resolve whether `repo` is covered by a `selected`-visibility
/// organization secret/variable, given already-classified `metadata` and
/// `repositories` read outcomes. Shared by the secret/variable resolvers
/// below; mirrors the analogous pattern used for `RepositoryAccessCategory`
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
    let Some(metadata) = record_read_outcome(
        coverage,
        ManifestCategoryName::Actions,
        metadata_endpoint,
        metadata,
    ) else {
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

    let Some(repositories) = record_read_outcome(
        coverage,
        ManifestCategoryName::Actions,
        repositories_endpoint,
        repositories,
    ) else {
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

pub(super) async fn resolve_org_secret_reference(
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

pub(super) async fn resolve_org_variable_reference(
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
