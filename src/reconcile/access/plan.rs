use super::collect::{actor_login, is_custom_repository_permission};
use super::{
    AccessCollection, AccessPlan, AccessReferenceAction, CollaboratorAccessAction,
    CollectedAccessReference, TeamAccessAction,
};
use crate::config::manifest::{
    ManagementDisposition, ReferencedResourceType, RepositoryAccessCategory,
};
use crate::reconcile::common::issue::{IssueSeverity, ReconcileIssue};
use std::collections::BTreeMap;

pub fn plan_access(current: &AccessCollection, desired: &RepositoryAccessCategory) -> AccessPlan {
    let mut team_actions = Vec::new();
    let mut collaborator_actions = Vec::new();
    let mut reference_actions = Vec::new();
    let mut notes = Vec::new();
    let mut issues = Vec::new();

    let current_teams = current
        .state
        .teams
        .iter()
        .cloned()
        .map(|team| (team.slug.clone(), team))
        .collect::<BTreeMap<_, _>>();
    let desired_teams = desired
        .desired_teams()
        .iter()
        .cloned()
        .map(|team| (team.slug.clone(), team))
        .collect::<BTreeMap<_, _>>();

    for desired_team in desired_teams.values() {
        if let Some(reference_issue) = missing_role_issue(
            &current.state.references,
            &desired_team.permission,
            format!("access.teams.{}", desired_team.slug),
        ) {
            issues.push(reference_issue);
            continue;
        }
        match current_teams.get(&desired_team.slug) {
            Some(current_team) if current_team.permission == desired_team.permission => {}
            _ => team_actions.push(TeamAccessAction::Ensure(desired_team.clone())),
        }
    }

    if desired.policy.prune && desired.teams.is_none() {
        notes.push(
            "Teams are not managed because `teams` is not set. Set `teams = []` to remove every team."
                .to_owned(),
        );
    }
    if desired.policy.prune && desired.teams.is_some() {
        if !current.state.teams_complete {
            issues.push(ReconcileIssue {
                scope: "access.teams".to_owned(),
                severity: IssueSeverity::Blocker,
                message: "Cannot safely prune team access because repository team collection was incomplete.".to_owned(),
            });
        } else {
            for current_team in current_teams.values() {
                if !desired_teams.contains_key(&current_team.slug) {
                    team_actions.push(TeamAccessAction::Remove(current_team.clone()));
                }
            }
        }
    }

    let current_collaborators = current
        .state
        .collaborators
        .iter()
        .cloned()
        .filter_map(|entry| {
            let login = actor_login(&entry.config.actor)?.to_owned();
            Some((login, entry))
        })
        .collect::<BTreeMap<_, _>>();
    let desired_collaborators = desired
        .desired_collaborators()
        .iter()
        .cloned()
        .filter_map(|entry| {
            let login = actor_login(&entry.actor)?.to_owned();
            Some((login, entry))
        })
        .collect::<BTreeMap<_, _>>();

    for desired_collaborator in desired_collaborators.values() {
        let Some(login) = actor_login(&desired_collaborator.actor) else {
            continue;
        };
        if let Some(reference_issue) = missing_role_issue(
            &current.state.references,
            &desired_collaborator.permission,
            format!("access.collaborators.{login}"),
        ) {
            issues.push(reference_issue);
            continue;
        }
        match current_collaborators.get(login) {
            Some(current_collaborator)
                if current_collaborator.pending
                    && current_collaborator.config.permission
                        == desired_collaborator.permission =>
            {
                notes.push(format!(
                    "Collaborator {login} already has a pending invitation with {} permission.",
                    desired_collaborator.permission
                ));
            }
            Some(current_collaborator) if current_collaborator.pending => {
                if let Some(invitation_id) = current_collaborator.invitation_id {
                    collaborator_actions.push(CollaboratorAccessAction::Reinvite {
                        invitation_id,
                        desired: desired_collaborator.clone(),
                    });
                } else {
                    issues.push(ReconcileIssue {
                        scope: format!("access.collaborators.{login}"),
                        severity: IssueSeverity::Blocker,
                        message: "Pending invitation exists but its invitation id is unknown; refusing to replace it.".to_owned(),
                    });
                }
            }
            Some(current_collaborator)
                if current_collaborator.config.permission != desired_collaborator.permission =>
            {
                collaborator_actions.push(CollaboratorAccessAction::Grant(
                    desired_collaborator.clone(),
                ));
            }
            None => collaborator_actions.push(CollaboratorAccessAction::Grant(
                desired_collaborator.clone(),
            )),
            _ => {}
        }
    }

    if desired.policy.prune && desired.collaborators.is_none() {
        notes.push(
            "Collaborators are not managed because `collaborators` is not set. Set `collaborators = []` to remove every collaborator."
                .to_owned(),
        );
    }
    if desired.policy.prune && desired.collaborators.is_some() {
        if !current.state.collaborators_complete {
            issues.push(ReconcileIssue {
                scope: "access.collaborators".to_owned(),
                severity: IssueSeverity::Blocker,
                message: "Cannot safely prune collaborators because collaborator collection was incomplete.".to_owned(),
            });
        } else {
            for current_collaborator in current_collaborators.values() {
                let Some(login) = actor_login(&current_collaborator.config.actor) else {
                    continue;
                };
                if !desired_collaborators.contains_key(login) {
                    collaborator_actions.push(CollaboratorAccessAction::Revoke {
                        login: login.to_owned(),
                        invitation_id: current_collaborator.invitation_id,
                    });
                }
            }
        }
    }

    for reference in &current.state.references {
        let scope = format!(
            "access.references.{}.{}",
            reference_kind_label(reference.resource.resource_type),
            reference.resource.name
        );
        match reference.resource.resource_type {
            ReferencedResourceType::OrganizationSecret
            | ReferencedResourceType::OrganizationVariable => match reference.present {
                Some(false) => issues.push(ReconcileIssue {
                    scope,
                    severity: IssueSeverity::Blocker,
                    message: format!(
                        "Referenced {:?} `{}` is missing.",
                        reference.resource.resource_type, reference.resource.name
                    ),
                }),
                None => issues.push(ReconcileIssue {
                    scope,
                    severity: IssueSeverity::Warning,
                    message: reference.detail.clone().unwrap_or_else(|| {
                        format!(
                            "Could not verify referenced {:?} `{}`.",
                            reference.resource.resource_type, reference.resource.name
                        )
                    }),
                }),
                Some(true) if !reference.supported => notes.push(format!(
                    "Observed {:?} {} without management: {}",
                    reference.resource.resource_type,
                    reference.resource.name,
                    reference.detail.clone().unwrap_or_else(|| {
                        "selected-repository association is not applicable".to_owned()
                    })
                )),
                Some(true) if matches!(reference.associated, Some(false)) => {
                    reference_actions
                        .push(AccessReferenceAction::Associate(reference.resource.clone()));
                }
                Some(true) if reference.associated.is_none() => issues.push(ReconcileIssue {
                    scope,
                    severity: IssueSeverity::Warning,
                    message: reference.detail.clone().unwrap_or_else(|| {
                        format!(
                            "Could not determine selected-repository association for {:?} `{}`.",
                            reference.resource.resource_type, reference.resource.name
                        )
                    }),
                }),
                _ => {}
            },
            _ => match reference.present {
                Some(false) => issues.push(ReconcileIssue {
                    scope,
                    severity: IssueSeverity::Blocker,
                    message: format!(
                        "Referenced {:?} `{}` is missing.",
                        reference.resource.resource_type, reference.resource.name
                    ),
                }),
                None => issues.push(ReconcileIssue {
                    scope,
                    severity: IssueSeverity::Warning,
                    message: reference.detail.clone().unwrap_or_else(|| {
                        format!(
                            "Could not verify referenced {:?} `{}`.",
                            reference.resource.resource_type, reference.resource.name
                        )
                    }),
                }),
                _ => {}
            },
        }
    }

    apply_access_policy_gates(
        desired,
        &mut team_actions,
        &mut collaborator_actions,
        &mut reference_actions,
        &mut issues,
    );

    AccessPlan {
        policy: desired.policy.clone(),
        team_actions,
        collaborator_actions,
        reference_actions,
        notes,
        issues,
    }
}

fn missing_role_issue(
    references: &[CollectedAccessReference],
    permission: &str,
    scope: String,
) -> Option<ReconcileIssue> {
    if !is_custom_repository_permission(permission) {
        return None;
    }
    references
        .iter()
        .find(|reference| {
            reference.resource.resource_type == ReferencedResourceType::Role
                && reference.resource.name == permission
        })
        .and_then(|reference| match reference.present {
            Some(true) => None,
            Some(false) => Some(ReconcileIssue {
                scope,
                severity: IssueSeverity::Blocker,
                message: format!("Custom repository role `{permission}` is missing."),
            }),
            None => Some(ReconcileIssue {
                scope,
                severity: IssueSeverity::Warning,
                message: reference.detail.clone().unwrap_or_else(|| {
                    format!("Could not verify custom repository role `{permission}`.")
                }),
            }),
        })
}

fn reference_kind_label(kind: ReferencedResourceType) -> &'static str {
    match kind {
        ReferencedResourceType::App => "app",
        ReferencedResourceType::Team => "team",
        ReferencedResourceType::Role => "role",
        ReferencedResourceType::RunnerGroup => "runner_group",
        ReferencedResourceType::OrganizationSecret => "org_secret",
        ReferencedResourceType::OrganizationVariable => "org_variable",
        ReferencedResourceType::CodeSecurityConfiguration => "code_security_configuration",
        ReferencedResourceType::ProtectionRule => "protection_rule",
        ReferencedResourceType::Runner => "runner",
    }
}

fn apply_access_policy_gates(
    desired: &RepositoryAccessCategory,
    team_actions: &mut Vec<TeamAccessAction>,
    collaborator_actions: &mut Vec<CollaboratorAccessAction>,
    reference_actions: &mut Vec<AccessReferenceAction>,
    issues: &mut Vec<ReconcileIssue>,
) {
    if desired.policy.disposition != ManagementDisposition::Managed {
        issues.extend(team_actions.iter().map(|action| ReconcileIssue {
            scope: format!("access.teams.{:?}", action),
            severity: IssueSeverity::Warning,
            message: "Team access change observed but access category is not managed.".to_owned(),
        }));
        issues.extend(collaborator_actions.iter().map(|action| ReconcileIssue {
            scope: format!("access.collaborators.{:?}", action),
            severity: IssueSeverity::Warning,
            message: "Collaborator change observed but access category is not managed.".to_owned(),
        }));
        issues.extend(reference_actions.iter().map(|action| ReconcileIssue {
            scope: format!("access.references.{:?}", action),
            severity: IssueSeverity::Warning,
            message:
                "Reference association observed but access category is not managed.".to_owned(),
        }));
        team_actions.clear();
        collaborator_actions.clear();
        reference_actions.clear();
        return;
    }

    if !desired.policy.sensitive {
        if !team_actions.is_empty()
            || !collaborator_actions.is_empty()
            || !reference_actions.is_empty()
        {
            issues.push(ReconcileIssue {
                scope: "access".to_owned(),
                severity: IssueSeverity::Blocker,
                message: "Access mutations require `policy.sensitive: true`.".to_owned(),
            });
        }
        team_actions.clear();
        collaborator_actions.clear();
        reference_actions.clear();
    }
}
