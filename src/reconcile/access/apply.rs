use super::collect::{actor_login, collect_access};
use super::{
    AccessApplyReport, AccessCollection, AccessPlan, AccessReferenceAction, AccessVerification,
    CollaboratorAccessAction, TeamAccessAction,
};
use crate::config::manifest::{
    ManagementDisposition, ReferencedResourceType, RepositoryAccessCategory,
};
use crate::github::Client;
use crate::github::access::CollaboratorGrantResult;
use crate::github::actions::WriteOutcome;
use crate::reconcile::common::issue::{IssueSeverity, format_issue};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

pub async fn apply_access(
    client: &Client,
    repo: &str,
    plan: &AccessPlan,
) -> Result<AccessApplyReport> {
    let mut report = AccessApplyReport::default();
    report.blocked.extend(
        plan.issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Blocker)
            .map(format_issue),
    );

    if plan.policy.disposition != ManagementDisposition::Managed || !plan.policy.sensitive {
        return Ok(report);
    }

    for action in &plan.team_actions {
        match action {
            TeamAccessAction::Ensure(team) => {
                client
                    .add_team_to_repo(repo, &team.slug, &team.permission)
                    .await?;
                report
                    .applied
                    .push(format!("Set team {} to {}", team.slug, team.permission));
            }
            TeamAccessAction::Remove(team) => {
                client.remove_team_from_repo(repo, &team.slug).await?;
                report.applied.push(format!("Removed team {}", team.slug));
            }
        }
    }

    for action in &plan.collaborator_actions {
        match action {
            CollaboratorAccessAction::Grant(config) => {
                let Some(login) = actor_login(&config.actor) else {
                    continue;
                };
                match client
                    .add_repo_collaborator(repo, login, &config.permission)
                    .await?
                {
                    CollaboratorGrantResult::Active => report.applied.push(format!(
                        "Granted collaborator {login} {}",
                        config.permission
                    )),
                    CollaboratorGrantResult::PendingInvitation => report.pending.push(format!(
                        "Collaborator {login} invitation is pending for {}",
                        config.permission
                    )),
                }
            }
            CollaboratorAccessAction::Reinvite {
                invitation_id,
                desired,
            } => {
                let Some(login) = actor_login(&desired.actor) else {
                    continue;
                };
                match client.cancel_repo_invitation(repo, *invitation_id).await? {
                    WriteOutcome::Applied(()) => match client
                        .add_repo_collaborator(repo, login, &desired.permission)
                        .await?
                    {
                        CollaboratorGrantResult::Active => report.applied.push(format!(
                            "Replaced pending invitation for {login} with {} access",
                            desired.permission
                        )),
                        CollaboratorGrantResult::PendingInvitation => report.pending.push(format!(
                            "Replaced pending invitation for {login}; new invitation is pending for {}",
                            desired.permission
                        )),
                    },
                    WriteOutcome::Blocked(reason) => report.blocked.push(format!(
                        "Failed to cancel pending invitation for {login}: {reason}"
                    )),
                }
            }
            CollaboratorAccessAction::Revoke {
                login,
                invitation_id,
            } => {
                let outcome = if let Some(invitation_id) = invitation_id {
                    client.cancel_repo_invitation(repo, *invitation_id).await?
                } else {
                    client.remove_repo_collaborator(repo, login).await?
                };
                match outcome {
                    WriteOutcome::Applied(()) => report
                        .applied
                        .push(format!("Removed collaborator/invitation {login}")),
                    WriteOutcome::Blocked(reason) => report.blocked.push(format!(
                        "Failed to remove collaborator/invitation {login}: {reason}"
                    )),
                }
            }
        }
    }

    for action in &plan.reference_actions {
        match action {
            AccessReferenceAction::Associate(reference) => {
                let outcome = match reference.resource_type {
                    ReferencedResourceType::OrganizationSecret => {
                        client
                            .associate_org_secret_with_repo(&reference.name, repo)
                            .await?
                    }
                    ReferencedResourceType::OrganizationVariable => {
                        client
                            .associate_org_variable_with_repo(&reference.name, repo)
                            .await?
                    }
                    _ => WriteOutcome::Blocked("reference type is observe-only".to_owned()),
                };
                match outcome {
                    WriteOutcome::Applied(()) => report.applied.push(format!(
                        "Associated {:?} {} with {repo}",
                        reference.resource_type, reference.name
                    )),
                    WriteOutcome::Blocked(reason) => report.blocked.push(format!(
                        "Failed to associate {:?} {} with {repo}: {reason}",
                        reference.resource_type, reference.name
                    )),
                }
            }
        }
    }

    Ok(report)
}

pub async fn verify_access(
    client: &Client,
    repo: &str,
    desired: &RepositoryAccessCategory,
) -> Result<AccessVerification> {
    let current = collect_access(client, repo, desired).await?;
    Ok(verify_access_state(&current, desired))
}

pub fn verify_access_state(
    current: &AccessCollection,
    desired: &RepositoryAccessCategory,
) -> AccessVerification {
    let mut verification = AccessVerification::default();
    let current_teams = current
        .state
        .teams
        .iter()
        .map(|team| (team.slug.as_str(), team))
        .collect::<BTreeMap<_, _>>();

    if current.state.teams_complete {
        for desired_team in desired.desired_teams() {
            match current_teams.get(desired_team.slug.as_str()) {
                None => verification.issues.push(format!(
                    "Missing team {} ({})",
                    desired_team.slug, desired_team.permission
                )),
                Some(current_team) if current_team.permission != desired_team.permission => {
                    verification.issues.push(format!(
                        "Team {} has {} instead of {}",
                        desired_team.slug, current_team.permission, desired_team.permission
                    ));
                }
                _ => {}
            }
        }
        if desired.policy.prune && desired.teams.is_some() {
            let desired_team_slugs = desired
                .desired_teams()
                .iter()
                .map(|team| team.slug.as_str())
                .collect::<BTreeSet<_>>();
            for current_team in &current.state.teams {
                if !desired_team_slugs.contains(current_team.slug.as_str()) {
                    verification.issues.push(format!(
                        "Unexpected team {} still has access",
                        current_team.slug
                    ));
                }
            }
        }
    } else if desired
        .teams
        .as_ref()
        .is_some_and(|teams| !teams.is_empty() || desired.policy.prune)
    {
        verification.notes.push(
            "Could not fully verify team access because repository team collection was incomplete."
                .to_owned(),
        );
    }

    let current_collaborators = current
        .state
        .collaborators
        .iter()
        .filter_map(|entry| actor_login(&entry.config.actor).map(|login| (login, entry)))
        .collect::<BTreeMap<_, _>>();

    for desired_collaborator in desired.desired_collaborators() {
        let Some(login) = actor_login(&desired_collaborator.actor) else {
            verification.notes.push(format!(
                "Skipping unsupported collaborator actor {:?}",
                desired_collaborator.actor
            ));
            continue;
        };

        if !current.state.collaborators_complete {
            verification.notes.push(format!(
                "Could not fully verify collaborator {} because collaborator collection was incomplete.",
                login
            ));
            continue;
        }

        match current_collaborators.get(login) {
            None => verification.issues.push(format!(
                "Missing collaborator {login} ({})",
                desired_collaborator.permission
            )),
            Some(current_collaborator)
                if current_collaborator.pending
                    && current_collaborator.config.permission
                        == desired_collaborator.permission =>
            {
                verification.pending.push(format!(
                    "Collaborator {login} invitation is still pending for {}",
                    desired_collaborator.permission
                ));
            }
            Some(current_collaborator)
                if current_collaborator.config.permission != desired_collaborator.permission =>
            {
                verification.issues.push(format!(
                    "Collaborator {login} has {} instead of {}",
                    current_collaborator.config.permission, desired_collaborator.permission
                ));
            }
            _ => {}
        }
    }

    if desired.policy.prune && desired.collaborators.is_some() {
        if current.state.collaborators_complete {
            let desired_logins = desired
                .desired_collaborators()
                .iter()
                .filter_map(|entry| actor_login(&entry.actor))
                .collect::<BTreeSet<_>>();
            for current_collaborator in &current.state.collaborators {
                let Some(login) = actor_login(&current_collaborator.config.actor) else {
                    continue;
                };
                if !desired_logins.contains(login) {
                    verification
                        .issues
                        .push(format!("Unexpected collaborator {login} still has access"));
                }
            }
        } else {
            verification.notes.push(
                "Could not verify collaborator prune because collaborator collection was incomplete."
                    .to_owned(),
            );
        }
    }

    for reference in &current.state.references {
        match reference.present {
            Some(false) => verification.issues.push(format!(
                "Referenced {:?} {} is missing",
                reference.resource.resource_type, reference.resource.name
            )),
            None => verification
                .notes
                .push(reference.detail.clone().unwrap_or_else(|| {
                    format!(
                        "Could not verify referenced {:?} {}",
                        reference.resource.resource_type, reference.resource.name
                    )
                })),
            Some(true)
                if matches!(
                    reference.resource.resource_type,
                    ReferencedResourceType::OrganizationSecret
                        | ReferencedResourceType::OrganizationVariable
                ) && reference.supported
                    && matches!(reference.associated, Some(false)) =>
            {
                verification.issues.push(format!(
                    "Referenced {:?} {} is not associated with the repository",
                    reference.resource.resource_type, reference.resource.name
                ));
            }
            _ => {}
        }
    }

    verification
}
