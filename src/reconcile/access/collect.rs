use super::{
    AccessCollection, CollectedAccessReference, CollectedAccessState, CollectedCollaborator,
};
use crate::config::manifest::{
    ActorReference, CollaboratorAccessConfig, CoverageEntry, ManifestCategoryName,
    ReferencedResourceConfig, ReferencedResourceType, RepositoryAccessCategory, TeamAccess,
};
use crate::github::Client;
use crate::github::access::{
    CollaboratorAffiliation, CustomRepositoryRole, NamedRepository, OrgScopedResourceMetadata,
    PendingCollaboratorInvitation, RepositoryAppInstallation, RepositoryCollaborator,
};
use crate::github::actions::ReadOutcome;
use crate::reconcile::access_integrations::record_read_outcome;
use anyhow::Result;
use std::collections::{BTreeSet, HashMap};

const BUILTIN_REPOSITORY_PERMISSIONS: &[&str] = &["pull", "triage", "push", "maintain", "admin"];

pub async fn collect_access(
    client: &Client,
    repo: &str,
    desired: &RepositoryAccessCategory,
) -> Result<AccessCollection> {
    let mut coverage = Vec::new();
    let issues = Vec::new();

    let (teams_outcome, direct_outcome, outside_outcome, invitations_outcome) = tokio::join!(
        client.list_repo_teams_checked(repo),
        client.list_repo_collaborators_checked(repo, CollaboratorAffiliation::Direct),
        client.list_repo_collaborators_checked(repo, CollaboratorAffiliation::Outside),
        client.list_repo_invitations_checked(repo),
    );

    let teams = match teams_outcome? {
        ReadOutcome::Available(teams) => teams.iter().map(TeamAccess::from).collect::<Vec<_>>(),
        outcome => {
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Access,
                &format!("/repos/{}/{repo}/teams", client.org()),
                outcome,
            );
            Vec::new()
        }
    };
    let teams_complete = !coverage
        .iter()
        .any(|entry| entry.endpoint.ends_with(&format!("/{repo}/teams")));

    let mut collaborators = Vec::new();
    let mut collaborators_complete = true;

    let direct = match direct_outcome? {
        ReadOutcome::Available(value) => value,
        outcome => {
            collaborators_complete = false;
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Access,
                &format!(
                    "/repos/{}/{repo}/collaborators?affiliation=direct",
                    client.org()
                ),
                outcome,
            );
            Vec::new()
        }
    };
    let outside = match outside_outcome? {
        ReadOutcome::Available(value) => value,
        outcome => {
            collaborators_complete = false;
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Access,
                &format!(
                    "/repos/{}/{repo}/collaborators?affiliation=outside",
                    client.org()
                ),
                outcome,
            );
            Vec::new()
        }
    };
    let invitations = match invitations_outcome? {
        ReadOutcome::Available(value) => value,
        outcome => {
            collaborators_complete = false;
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Access,
                &format!("/repos/{}/{repo}/invitations", client.org()),
                outcome,
            );
            Vec::new()
        }
    };
    collaborators.extend(collect_collaborators(direct, outside, invitations));

    let app_installations = match client.list_repo_app_installations_checked(repo).await? {
        ReadOutcome::Available(installations) => Some(installations),
        outcome => {
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Access,
                &format!("/user/installations[repo={repo}]"),
                outcome,
            );
            None
        }
    };
    let imported_app_refs = app_installations
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|installation| ReferencedResourceConfig {
            resource_type: ReferencedResourceType::App,
            name: installation.app_slug.clone(),
        })
        .collect::<Vec<_>>();
    let derived_refs =
        derive_access_references(desired, &teams, &collaborators, &imported_app_refs);
    let references = collect_access_references(
        client,
        repo,
        &derived_refs,
        app_installations.as_deref(),
        &mut coverage,
    )
    .await?;
    let category = RepositoryAccessCategory {
        policy: desired.policy.clone(),
        teams: teams_complete.then(|| teams.clone()),
        collaborators: collaborators_complete.then(|| {
            collaborators
                .iter()
                .map(|entry| entry.config.clone())
                .collect()
        }),
        references: derived_refs,
    };

    Ok(AccessCollection {
        category,
        state: CollectedAccessState {
            teams,
            teams_complete,
            collaborators,
            collaborators_complete,
            references,
        },
        coverage,
        issues,
    })
}

async fn collect_access_references(
    client: &Client,
    repo: &str,
    references: &[ReferencedResourceConfig],
    app_installations: Option<&[RepositoryAppInstallation]>,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<Vec<CollectedAccessReference>> {
    let need_team_lookup = references
        .iter()
        .any(|reference| matches!(reference.resource_type, ReferencedResourceType::Team));
    let need_role_lookup = references
        .iter()
        .any(|reference| matches!(reference.resource_type, ReferencedResourceType::Role));
    let org_teams = if need_team_lookup {
        record_read_outcome(
            coverage,
            ManifestCategoryName::Access,
            &format!("/orgs/{}/teams", client.org()),
            client.list_org_teams_checked().await?,
        )
    } else {
        None
    };
    let custom_roles = if need_role_lookup {
        record_read_outcome(
            coverage,
            ManifestCategoryName::Access,
            &format!("/orgs/{}/custom-repository-roles", client.org()),
            client.list_custom_repository_roles_checked().await?,
        )
    } else {
        None
    };

    let mut collected = Vec::new();
    for reference in references {
        let state = match reference.resource_type {
            ReferencedResourceType::Team => collect_team_reference(reference, org_teams.as_deref()),
            ReferencedResourceType::Role => {
                collect_role_reference(reference, custom_roles.as_deref())
            }
            ReferencedResourceType::App => collect_app_reference(reference, app_installations),
            ReferencedResourceType::OrganizationSecret => {
                collect_org_secret_reference(client, repo, reference, coverage).await?
            }
            ReferencedResourceType::OrganizationVariable => {
                collect_org_variable_reference(client, repo, reference, coverage).await?
            }
            _ => CollectedAccessReference {
                resource: reference.clone(),
                present: Some(true),
                associated: None,
                supported: false,
                detail: Some(
                    "This reference type is observe-only in access reconciliation.".to_owned(),
                ),
            },
        };
        collected.push(state);
    }

    Ok(collected)
}

fn collect_team_reference(
    reference: &ReferencedResourceConfig,
    teams: Option<&[crate::github::teams::Team]>,
) -> CollectedAccessReference {
    match teams {
        Some(teams) => CollectedAccessReference {
            resource: reference.clone(),
            present: Some(
                teams
                    .iter()
                    .any(|team| team.slug == reference.name || team.name == reference.name),
            ),
            associated: None,
            supported: true,
            detail: None,
        },
        None => CollectedAccessReference {
            resource: reference.clone(),
            present: None,
            associated: None,
            supported: true,
            detail: Some("Organization team lookup was unavailable.".to_owned()),
        },
    }
}

fn collect_role_reference(
    reference: &ReferencedResourceConfig,
    roles: Option<&[CustomRepositoryRole]>,
) -> CollectedAccessReference {
    match roles {
        Some(roles) => CollectedAccessReference {
            resource: reference.clone(),
            present: Some(roles.iter().any(|role| role.name == reference.name)),
            associated: None,
            supported: true,
            detail: None,
        },
        None => CollectedAccessReference {
            resource: reference.clone(),
            present: None,
            associated: None,
            supported: true,
            detail: Some("Custom repository role lookup was unavailable.".to_owned()),
        },
    }
}

fn collect_app_reference(
    reference: &ReferencedResourceConfig,
    installations: Option<&[RepositoryAppInstallation]>,
) -> CollectedAccessReference {
    match installations {
        Some(installations) => CollectedAccessReference {
            resource: reference.clone(),
            present: Some(
                installations
                    .iter()
                    .any(|installation| installation.app_slug == reference.name),
            ),
            associated: None,
            supported: true,
            detail: None,
        },
        None => CollectedAccessReference {
            resource: reference.clone(),
            present: None,
            associated: None,
            supported: true,
            detail: Some("GitHub App installation lookup was unavailable.".to_owned()),
        },
    }
}

async fn collect_org_secret_reference(
    client: &Client,
    repo: &str,
    reference: &ReferencedResourceConfig,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<CollectedAccessReference> {
    Ok(collect_selected_repository_reference(
        repo,
        reference,
        (
            &format!("/orgs/{}/actions/secrets/{}", client.org(), reference.name),
            &format!(
                "/orgs/{}/actions/secrets/{}/repositories",
                client.org(),
                reference.name
            ),
        ),
        client
            .get_org_secret_metadata_checked(&reference.name)
            .await?,
        client
            .list_org_secret_selected_repositories_checked(&reference.name)
            .await?,
        coverage,
    ))
}

async fn collect_org_variable_reference(
    client: &Client,
    repo: &str,
    reference: &ReferencedResourceConfig,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<CollectedAccessReference> {
    Ok(collect_selected_repository_reference(
        repo,
        reference,
        (
            &format!(
                "/orgs/{}/actions/variables/{}",
                client.org(),
                reference.name
            ),
            &format!(
                "/orgs/{}/actions/variables/{}/repositories",
                client.org(),
                reference.name
            ),
        ),
        client
            .get_org_variable_metadata_checked(&reference.name)
            .await?,
        client
            .list_org_variable_selected_repositories_checked(&reference.name)
            .await?,
        coverage,
    ))
}

fn collect_selected_repository_reference(
    repo: &str,
    reference: &ReferencedResourceConfig,
    endpoints: (&str, &str),
    metadata: ReadOutcome<Option<OrgScopedResourceMetadata>>,
    repositories: ReadOutcome<Option<Vec<NamedRepository>>>,
    coverage: &mut Vec<CoverageEntry>,
) -> CollectedAccessReference {
    let (metadata_endpoint, repositories_endpoint) = endpoints;
    let metadata = match metadata {
        ReadOutcome::Available(value) => value,
        outcome => {
            record_read_outcome(
                coverage,
                ManifestCategoryName::Access,
                metadata_endpoint,
                outcome,
            );
            return CollectedAccessReference {
                resource: reference.clone(),
                present: None,
                associated: None,
                supported: true,
                detail: Some("Referenced organization resource lookup was unavailable.".to_owned()),
            };
        }
    };

    let Some(metadata) = metadata else {
        return CollectedAccessReference {
            resource: reference.clone(),
            present: Some(false),
            associated: None,
            supported: true,
            detail: Some("Referenced organization resource was not found.".to_owned()),
        };
    };

    if metadata.visibility.as_deref() != Some("selected") {
        return CollectedAccessReference {
            resource: reference.clone(),
            present: Some(true),
            associated: None,
            supported: false,
            detail: Some(format!(
                "{} visibility is {:?}; selected-repository association is not applicable.",
                reference.name, metadata.visibility
            )),
        };
    }

    let repositories = match repositories {
        ReadOutcome::Available(value) => value,
        outcome => {
            record_read_outcome(
                coverage,
                ManifestCategoryName::Access,
                repositories_endpoint,
                outcome,
            );
            return CollectedAccessReference {
                resource: reference.clone(),
                present: Some(true),
                associated: None,
                supported: true,
                detail: Some("Selected-repository association lookup was unavailable.".to_owned()),
            };
        }
    };

    CollectedAccessReference {
        resource: reference.clone(),
        present: Some(true),
        associated: repositories
            .as_deref()
            .map(|repositories| repositories.iter().any(|entry| entry.name == repo)),
        supported: true,
        detail: None,
    }
}

fn collect_collaborators(
    direct: Vec<RepositoryCollaborator>,
    outside: Vec<RepositoryCollaborator>,
    pending: Vec<PendingCollaboratorInvitation>,
) -> Vec<CollectedCollaborator> {
    let mut collaborators = HashMap::new();
    for collaborator in direct {
        collaborators.insert(
            collaborator.login.clone(),
            CollectedCollaborator {
                config: CollaboratorAccessConfig {
                    actor: ActorReference::User {
                        login: collaborator.login.clone(),
                    },
                    permission: collaborator.permission,
                },
                outside: collaborator.outside,
                pending: false,
                invitation_id: None,
            },
        );
    }

    for collaborator in outside {
        collaborators
            .entry(collaborator.login.clone())
            .and_modify(|entry: &mut CollectedCollaborator| entry.outside = true)
            .or_insert_with(|| CollectedCollaborator {
                config: CollaboratorAccessConfig {
                    actor: ActorReference::User {
                        login: collaborator.login.clone(),
                    },
                    permission: collaborator.permission,
                },
                outside: true,
                pending: false,
                invitation_id: None,
            });
    }

    for invitation in pending {
        let outside = collaborators
            .get(&invitation.login)
            .map(|entry| entry.outside)
            .unwrap_or(false);
        collaborators.insert(
            invitation.login.clone(),
            CollectedCollaborator {
                config: CollaboratorAccessConfig {
                    actor: ActorReference::User {
                        login: invitation.login.clone(),
                    },
                    permission: invitation.permission,
                },
                outside,
                pending: true,
                invitation_id: Some(invitation.id),
            },
        );
    }

    let mut collaborators = collaborators.into_values().collect::<Vec<_>>();
    collaborators.sort_by(|left, right| {
        actor_login(&left.config.actor).cmp(&actor_login(&right.config.actor))
    });
    collaborators
}

pub(super) fn actor_login(actor: &ActorReference) -> Option<&str> {
    match actor {
        ActorReference::User { login } => Some(login.as_str()),
        _ => None,
    }
}

fn derive_access_references(
    desired: &RepositoryAccessCategory,
    teams: &[TeamAccess],
    collaborators: &[CollectedCollaborator],
    app_refs: &[ReferencedResourceConfig],
) -> Vec<ReferencedResourceConfig> {
    let mut seen = BTreeSet::new();
    let mut references = Vec::new();
    for reference in &desired.references {
        push_reference(&mut references, &mut seen, reference.clone());
    }
    for team in teams {
        if is_custom_repository_permission(&team.permission) {
            push_reference(
                &mut references,
                &mut seen,
                ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::Role,
                    name: team.permission.clone(),
                },
            );
        }
    }
    for collaborator in collaborators {
        if is_custom_repository_permission(&collaborator.config.permission) {
            push_reference(
                &mut references,
                &mut seen,
                ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::Role,
                    name: collaborator.config.permission.clone(),
                },
            );
        }
    }
    for reference in app_refs {
        push_reference(&mut references, &mut seen, reference.clone());
    }
    references
}

fn push_reference(
    references: &mut Vec<ReferencedResourceConfig>,
    seen: &mut BTreeSet<String>,
    reference: ReferencedResourceConfig,
) {
    let key = format!("{:?}:{}", reference.resource_type, reference.name);
    if seen.insert(key) {
        references.push(reference);
    }
}

pub(super) fn is_custom_repository_permission(permission: &str) -> bool {
    !BUILTIN_REPOSITORY_PERMISSIONS.contains(&permission)
}
