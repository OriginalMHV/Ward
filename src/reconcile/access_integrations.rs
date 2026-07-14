//! Access and integration snapshot/reconciliation.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;

use anyhow::Result;

use crate::config::manifest::{
    ActorReference, AutolinkConfigV2, CategoryPolicy, CollaboratorAccessConfig, CoverageEntry,
    CoverageOutcome, DeployKeyConfigV2, ExternalValueReference, ManagementDisposition,
    ManifestCategoryName, PagesConfigV2, ReferencedResourceConfig, ReferencedResourceType,
    RepositoryAccessCategoryV2, RepositoryIntegrationsCategoryV2, TeamAccess, WebhookConfigV2,
};
use crate::github::Client;
use crate::github::access::{
    CollaboratorAffiliation, CollaboratorGrantResult, CustomRepositoryRole, NamedRepository,
    OrgScopedResourceMetadata, PendingCollaboratorInvitation, RepositoryAppInstallation,
    RepositoryCollaborator,
};
use crate::github::actions::{ReadOutcome, WriteOutcome};
use crate::github::integrations::{WebhookConfigPatch, WebhookMetadataPatch};
use crate::reconcile::actions_environments::{IssueSeverity, ReconcileIssue};

const WEBHOOK_SECRET_HINT: &str =
    "GitHub does not return existing webhook secret values; preserve or rotate it explicitly.";
const DEPLOY_KEY_HINT: &str =
    "GitHub does not return deploy key material; provide replacement_key to rotate this key.";
const WEBHOOK_URL_ENV_PREFIX: &str = "WARD_WEBHOOK_URL_";
const BUILTIN_REPOSITORY_PERMISSIONS: &[&str] = &["pull", "triage", "push", "maintain", "admin"];

#[derive(Debug, Clone, PartialEq)]
pub struct AccessCollection {
    pub category: RepositoryAccessCategoryV2,
    pub state: CollectedAccessState,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedAccessState {
    pub teams: Vec<TeamAccess>,
    pub teams_complete: bool,
    pub collaborators: Vec<CollectedCollaborator>,
    pub collaborators_complete: bool,
    pub references: Vec<CollectedAccessReference>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedCollaborator {
    pub config: CollaboratorAccessConfig,
    pub outside: bool,
    pub pending: bool,
    pub invitation_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedAccessReference {
    pub resource: ReferencedResourceConfig,
    pub present: Option<bool>,
    pub associated: Option<bool>,
    pub supported: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessPlan {
    pub policy: CategoryPolicy,
    pub team_actions: Vec<TeamAccessAction>,
    pub collaborator_actions: Vec<CollaboratorAccessAction>,
    pub reference_actions: Vec<AccessReferenceAction>,
    pub notes: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

impl AccessPlan {
    pub fn is_empty(&self) -> bool {
        self.team_actions.is_empty()
            && self.collaborator_actions.is_empty()
            && self.reference_actions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TeamAccessAction {
    Ensure(TeamAccess),
    Remove(TeamAccess),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CollaboratorAccessAction {
    Grant(CollaboratorAccessConfig),
    Reinvite {
        invitation_id: u64,
        desired: CollaboratorAccessConfig,
    },
    Revoke {
        login: String,
        invitation_id: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccessReferenceAction {
    Associate(ReferencedResourceConfig),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccessApplyReport {
    pub applied: Vec<String>,
    pub pending: Vec<String>,
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccessVerification {
    pub issues: Vec<String>,
    pub pending: Vec<String>,
    pub notes: Vec<String>,
}

impl AccessVerification {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationsCollection {
    pub category: RepositoryIntegrationsCategoryV2,
    pub state: CollectedIntegrationsState,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedIntegrationsState {
    pub webhooks: Vec<CollectedWebhook>,
    pub webhooks_complete: bool,
    pub deploy_keys: Vec<CollectedDeployKey>,
    pub deploy_keys_complete: bool,
    pub pages: Option<CollectedPages>,
    pub pages_complete: bool,
    pub autolinks: Vec<CollectedAutolink>,
    pub autolinks_complete: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedWebhook {
    pub id: u64,
    pub config: WebhookConfigV2,
    pub canonical_url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedDeployKey {
    pub id: u64,
    pub config: DeployKeyConfigV2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedPages {
    pub config: PagesConfigV2,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectedAutolink {
    pub id: u64,
    pub config: AutolinkConfigV2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationsPlan {
    pub policy: CategoryPolicy,
    pub webhook_actions: Vec<WebhookAction>,
    pub deploy_key_actions: Vec<DeployKeyAction>,
    pub pages_action: Option<PagesAction>,
    pub autolink_actions: Vec<AutolinkAction>,
    pub notes: Vec<String>,
    pub issues: Vec<ReconcileIssue>,
}

impl IntegrationsPlan {
    pub fn is_empty(&self) -> bool {
        self.webhook_actions.is_empty()
            && self.deploy_key_actions.is_empty()
            && self.pages_action.is_none()
            && self.autolink_actions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebhookAction {
    Create(WebhookConfigV2),
    Update {
        hook_id: u64,
        current: CollectedWebhook,
        desired: WebhookConfigV2,
    },
    Delete {
        hook_id: u64,
        redacted_url: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeployKeyAction {
    Create(DeployKeyConfigV2),
    Replace {
        key_id: u64,
        current_title: String,
        desired: DeployKeyConfigV2,
    },
    Delete {
        key_id: u64,
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PagesAction {
    Create(PagesConfigV2),
    Update(PagesConfigV2),
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AutolinkAction {
    Create(AutolinkConfigV2),
    Recreate {
        autolink_id: u64,
        desired: AutolinkConfigV2,
    },
    Delete {
        autolink_id: u64,
        key_prefix: String,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IntegrationsApplyReport {
    pub applied: Vec<String>,
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IntegrationsVerification {
    pub issues: Vec<String>,
    pub notes: Vec<String>,
}

impl IntegrationsVerification {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

pub async fn collect_access(
    client: &Client,
    repo: &str,
    desired: &RepositoryAccessCategoryV2,
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
    let category = RepositoryAccessCategoryV2 {
        policy: desired.policy.clone(),
        teams: teams.clone(),
        collaborators: collaborators
            .iter()
            .map(|entry| entry.config.clone())
            .collect(),
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

pub fn plan_access(current: &AccessCollection, desired: &RepositoryAccessCategoryV2) -> AccessPlan {
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
        .teams
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

    if desired.policy.prune {
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
        .collaborators
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

    if desired.policy.prune {
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
    desired: &RepositoryAccessCategoryV2,
) -> Result<AccessVerification> {
    let current = collect_access(client, repo, desired).await?;
    Ok(verify_access_state(&current, desired))
}

pub fn verify_access_state(
    current: &AccessCollection,
    desired: &RepositoryAccessCategoryV2,
) -> AccessVerification {
    let mut verification = AccessVerification::default();
    let current_teams = current
        .state
        .teams
        .iter()
        .map(|team| (team.slug.as_str(), team))
        .collect::<BTreeMap<_, _>>();

    if current.state.teams_complete {
        for desired_team in &desired.teams {
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
        if desired.policy.prune {
            let desired_team_slugs = desired
                .teams
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
    } else if !desired.teams.is_empty() || desired.policy.prune {
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

    for desired_collaborator in &desired.collaborators {
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

    if desired.policy.prune {
        if current.state.collaborators_complete {
            let desired_logins = desired
                .collaborators
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

pub async fn collect_integrations(
    client: &Client,
    repo: &str,
    desired: &RepositoryIntegrationsCategoryV2,
) -> Result<IntegrationsCollection> {
    let mut coverage = Vec::new();
    let issues = Vec::new();
    let (webhooks_outcome, deploy_keys_outcome, pages_outcome, autolinks_outcome) = tokio::join!(
        client.list_repo_webhooks_checked(repo),
        client.list_repo_deploy_keys_checked(repo),
        client.get_repo_pages_checked(repo),
        client.list_repo_autolinks_checked(repo),
    );

    let (webhooks, webhooks_complete) = match webhooks_outcome? {
        ReadOutcome::Available(webhooks) => (
            webhooks
                .into_iter()
                .map(|webhook| {
                    let (display_url, url_from) = imported_webhook_identity(&webhook.url);
                    CollectedWebhook {
                        id: webhook.id,
                        canonical_url: canonicalize_url(&display_url),
                        config: WebhookConfigV2 {
                            url: display_url,
                            url_from,
                            active: Some(webhook.active),
                            events: normalize_events(&webhook.events),
                            content_type: webhook.content_type,
                            insecure_ssl: webhook.insecure_ssl,
                            secret: Some(ExternalValueReference::Manual {
                                hint: Some(WEBHOOK_SECRET_HINT.to_owned()),
                            }),
                        },
                    }
                })
                .collect(),
            true,
        ),
        outcome => {
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Integrations,
                &format!("/repos/{}/{repo}/hooks", client.org()),
                outcome,
            );
            (Vec::new(), false)
        }
    };

    let (deploy_keys, deploy_keys_complete) = match deploy_keys_outcome? {
        ReadOutcome::Available(keys) => (
            keys.into_iter()
                .map(|key| CollectedDeployKey {
                    id: key.id,
                    config: DeployKeyConfigV2 {
                        title: key.title,
                        read_only: Some(key.read_only),
                        fingerprint: key.fingerprint,
                        replacement_key: Some(ExternalValueReference::Manual {
                            hint: Some(DEPLOY_KEY_HINT.to_owned()),
                        }),
                    },
                })
                .collect(),
            true,
        ),
        outcome => {
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Integrations,
                &format!("/repos/{}/{repo}/keys", client.org()),
                outcome,
            );
            (Vec::new(), false)
        }
    };

    let (pages, pages_complete) = match pages_outcome? {
        ReadOutcome::Available(Some(pages)) => (
            Some(CollectedPages {
                config: PagesConfigV2 {
                    build_type: pages.build_type,
                    source_branch: pages.source_branch,
                    source_path: pages.source_path,
                    cname: pages.cname,
                    https_enforced: pages.https_enforced,
                },
                status: pages.status,
            }),
            true,
        ),
        ReadOutcome::Available(None) => (None, true),
        outcome => {
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Integrations,
                &format!("/repos/{}/{repo}/pages", client.org()),
                outcome,
            );
            (None, false)
        }
    };

    let (autolinks, autolinks_complete) = match autolinks_outcome? {
        ReadOutcome::Available(autolinks) => (
            autolinks
                .into_iter()
                .map(|autolink| CollectedAutolink {
                    id: autolink.id,
                    config: AutolinkConfigV2 {
                        key_prefix: autolink.key_prefix,
                        url_template: autolink.url_template,
                        is_alphanumeric: autolink.is_alphanumeric,
                    },
                })
                .collect(),
            true,
        ),
        outcome => {
            record_read_outcome(
                &mut coverage,
                ManifestCategoryName::Integrations,
                &format!("/repos/{}/{repo}/autolinks", client.org()),
                outcome,
            );
            (Vec::new(), false)
        }
    };

    let category = RepositoryIntegrationsCategoryV2 {
        policy: desired.policy.clone(),
        webhooks: webhooks.iter().map(|hook| hook.config.clone()).collect(),
        deploy_keys: deploy_keys.iter().map(|key| key.config.clone()).collect(),
        pages: pages.as_ref().map(|entry| entry.config.clone()),
        autolinks: autolinks.iter().map(|entry| entry.config.clone()).collect(),
        labels: Vec::new(),
    };

    Ok(IntegrationsCollection {
        category,
        state: CollectedIntegrationsState {
            webhooks,
            webhooks_complete,
            deploy_keys,
            deploy_keys_complete,
            pages,
            pages_complete,
            autolinks,
            autolinks_complete,
        },
        coverage,
        issues,
    })
}

pub fn plan_integrations(
    current: &IntegrationsCollection,
    desired: &RepositoryIntegrationsCategoryV2,
) -> IntegrationsPlan {
    let mut webhook_actions = Vec::new();
    let mut deploy_key_actions = Vec::new();
    let mut autolink_actions = Vec::new();
    let mut notes = Vec::new();
    let mut issues = Vec::new();

    let current_webhooks = current
        .state
        .webhooks
        .iter()
        .map(|hook| (hook.canonical_url.clone(), hook))
        .collect::<BTreeMap<_, _>>();
    let desired_webhooks = desired
        .webhooks
        .iter()
        .cloned()
        .map(|hook| (canonicalize_url(&hook.url), hook))
        .collect::<BTreeMap<_, _>>();

    if !current.state.webhooks_complete && (!desired.webhooks.is_empty() || desired.policy.prune) {
        issues.push(ReconcileIssue {
            scope: "integrations.webhooks".to_owned(),
            severity: IssueSeverity::Blocker,
            message: "Cannot safely manage webhooks because webhook collection was incomplete."
                .to_owned(),
        });
    } else {
        for (canonical_url, desired_hook) in &desired_webhooks {
            match current_webhooks.get(canonical_url) {
                None => {
                    if let Some(reason) = webhook_create_block_reason(desired_hook) {
                        issues.push(ReconcileIssue {
                            scope: format!("integrations.webhooks.{canonical_url}"),
                            severity: IssueSeverity::Blocker,
                            message: reason,
                        });
                    } else {
                        webhook_actions.push(WebhookAction::Create(desired_hook.clone()));
                    }
                }
                Some(current_hook)
                    if normalized_webhook(current_hook)
                        != normalized_webhook_config(desired_hook) =>
                {
                    if let Some(reason) = webhook_update_block_reason(current_hook, desired_hook) {
                        issues.push(ReconcileIssue {
                            scope: format!("integrations.webhooks.{canonical_url}"),
                            severity: IssueSeverity::Blocker,
                            message: reason,
                        });
                    } else {
                        webhook_actions.push(WebhookAction::Update {
                            hook_id: current_hook.id,
                            current: (*current_hook).clone(),
                            desired: desired_hook.clone(),
                        });
                    }
                }
                _ => {}
            }
        }

        if desired.policy.prune {
            for (canonical_url, current_hook) in &current_webhooks {
                if !desired_webhooks.contains_key(canonical_url) {
                    webhook_actions.push(WebhookAction::Delete {
                        hook_id: current_hook.id,
                        redacted_url: current_hook.config.url.clone(),
                    });
                }
            }
        }
    }

    let mut used_deploy_key_ids = BTreeSet::new();
    if !current.state.deploy_keys_complete
        && (!desired.deploy_keys.is_empty() || desired.policy.prune)
    {
        issues.push(ReconcileIssue {
            scope: "integrations.deploy_keys".to_owned(),
            severity: IssueSeverity::Blocker,
            message:
                "Cannot safely manage deploy keys because deploy-key collection was incomplete."
                    .to_owned(),
        });
    } else {
        for desired_key in &desired.deploy_keys {
            if let Some((id, current_key)) = match_deploy_key(&current.state, desired_key) {
                used_deploy_key_ids.insert(id);
                if deploy_key_equivalent(current_key, desired_key) {
                    continue;
                }
                if let Some(reason) = deploy_key_block_reason(desired_key) {
                    issues.push(ReconcileIssue {
                        scope: format!("integrations.deploy_keys.{}", desired_key.title),
                        severity: IssueSeverity::Blocker,
                        message: reason,
                    });
                } else {
                    deploy_key_actions.push(DeployKeyAction::Replace {
                        key_id: id,
                        current_title: current_key.config.title.clone(),
                        desired: desired_key.clone(),
                    });
                }
            } else if let Some(reason) = deploy_key_block_reason(desired_key) {
                issues.push(ReconcileIssue {
                    scope: format!("integrations.deploy_keys.{}", desired_key.title),
                    severity: IssueSeverity::Blocker,
                    message: reason,
                });
            } else {
                deploy_key_actions.push(DeployKeyAction::Create(desired_key.clone()));
            }
        }

        if desired.policy.prune {
            for current_key in &current.state.deploy_keys {
                if !used_deploy_key_ids.contains(&current_key.id) {
                    deploy_key_actions.push(DeployKeyAction::Delete {
                        key_id: current_key.id,
                        title: current_key.config.title.clone(),
                    });
                }
            }
        }
    }

    let pages_action = if !current.state.pages_complete
        && (desired.pages.is_some() || desired.policy.prune)
    {
        issues.push(ReconcileIssue {
            scope: "integrations.pages".to_owned(),
            severity: IssueSeverity::Blocker,
            message: "Cannot safely manage GitHub Pages because Pages collection was incomplete."
                .to_owned(),
        });
        None
    } else {
        let validation_issues = validate_pages_desired(desired.pages.as_ref());
        issues.extend(validation_issues);
        match (&current.state.pages, &desired.pages) {
            (None, Some(desired_pages))
                if !has_blocker_for_scope(&issues, "integrations.pages") =>
            {
                Some(PagesAction::Create(desired_pages.clone()))
            }
            (Some(_), None) if desired.policy.prune => Some(PagesAction::Delete),
            (Some(current_pages), Some(desired_pages))
                if normalized_pages(&current_pages.config) != normalized_pages(desired_pages)
                    && !has_blocker_for_scope(&issues, "integrations.pages") =>
            {
                Some(PagesAction::Update(desired_pages.clone()))
            }
            _ => None,
        }
    };

    let current_autolinks = current
        .state
        .autolinks
        .iter()
        .map(|autolink| (autolink.config.key_prefix.clone(), autolink))
        .collect::<BTreeMap<_, _>>();
    let desired_autolinks = desired
        .autolinks
        .iter()
        .cloned()
        .map(|autolink| (autolink.key_prefix.clone(), autolink))
        .collect::<BTreeMap<_, _>>();

    if !current.state.autolinks_complete && (!desired.autolinks.is_empty() || desired.policy.prune)
    {
        issues.push(ReconcileIssue {
            scope: "integrations.autolinks".to_owned(),
            severity: IssueSeverity::Blocker,
            message: "Cannot safely manage autolinks because autolink collection was incomplete."
                .to_owned(),
        });
    } else {
        for (key_prefix, desired_autolink) in &desired_autolinks {
            match current_autolinks.get(key_prefix) {
                None => autolink_actions.push(AutolinkAction::Create(desired_autolink.clone())),
                Some(current_autolink)
                    if normalized_autolink(&current_autolink.config)
                        != normalized_autolink(desired_autolink) =>
                {
                    autolink_actions.push(AutolinkAction::Recreate {
                        autolink_id: current_autolink.id,
                        desired: desired_autolink.clone(),
                    });
                }
                _ => {}
            }
        }

        if desired.policy.prune {
            for (key_prefix, current_autolink) in &current_autolinks {
                if !desired_autolinks.contains_key(key_prefix) {
                    autolink_actions.push(AutolinkAction::Delete {
                        autolink_id: current_autolink.id,
                        key_prefix: current_autolink.config.key_prefix.clone(),
                    });
                }
            }
        }
    }

    if !desired.labels.is_empty() {
        notes.push(
            "Labels remain owned by general-settings and are intentionally ignored here."
                .to_owned(),
        );
    }

    let mut pages_action = pages_action;
    apply_integrations_policy_gates(
        desired,
        &mut webhook_actions,
        &mut deploy_key_actions,
        &mut pages_action,
        &mut autolink_actions,
        &mut issues,
    );

    IntegrationsPlan {
        policy: desired.policy.clone(),
        webhook_actions,
        deploy_key_actions,
        pages_action,
        autolink_actions,
        notes,
        issues,
    }
}

pub async fn apply_integrations(
    client: &Client,
    repo: &str,
    plan: &IntegrationsPlan,
) -> Result<IntegrationsApplyReport> {
    let mut report = IntegrationsApplyReport::default();
    report.blocked.extend(
        plan.issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Blocker)
            .map(format_issue),
    );

    if plan.policy.disposition != ManagementDisposition::Managed {
        return Ok(report);
    }

    for action in &plan.webhook_actions {
        match action {
            WebhookAction::Create(desired) => {
                let resolved_url = match resolve_required_url(desired) {
                    Ok(url) => url,
                    Err(reason) => {
                        report
                            .blocked
                            .push(format!("Webhook {} was blocked: {reason}", desired.url));
                        continue;
                    }
                };
                let secret = match resolve_optional_external_value(desired.secret.as_ref()) {
                    Ok(secret) => secret,
                    Err(reason) => {
                        report
                            .blocked
                            .push(format!("Webhook {} was blocked: {reason}", desired.url));
                        continue;
                    }
                };
                client
                    .create_repo_webhook(repo, desired, &resolved_url, secret.as_deref())
                    .await?;
                report
                    .applied
                    .push(format!("Created webhook {}", desired.url));
            }
            WebhookAction::Update {
                hook_id,
                current,
                desired,
            } => {
                let desired_normalized = normalized_webhook_config(desired);
                let current_normalized = normalized_webhook(current);
                let url_patch =
                    if current_normalized.canonical_url != desired_normalized.canonical_url {
                        match resolve_required_url(desired) {
                            Ok(url) => Some(url),
                            Err(reason) => {
                                report
                                    .blocked
                                    .push(format!("Webhook {} was blocked: {reason}", desired.url));
                                continue;
                            }
                        }
                    } else {
                        None
                    };
                let config_patch = WebhookConfigPatch {
                    url: url_patch.as_deref(),
                    content_type: (current.config.content_type != desired.content_type)
                        .then_some(desired.content_type.as_deref())
                        .flatten(),
                    insecure_ssl: (current.config.insecure_ssl != desired.insecure_ssl)
                        .then_some(desired.insecure_ssl)
                        .flatten(),
                    secret: None,
                };
                let metadata_patch = WebhookMetadataPatch {
                    active: (current.config.active.unwrap_or(true)
                        != desired.active.unwrap_or(true))
                    .then_some(desired.active.unwrap_or(true)),
                    events: (normalize_events(&current.config.events)
                        != normalize_events(&desired.events))
                    .then_some(desired.events.as_slice()),
                };
                client
                    .update_repo_webhook(repo, *hook_id, config_patch, metadata_patch)
                    .await?;
                report
                    .applied
                    .push(format!("Updated webhook {}", desired.url));
            }
            WebhookAction::Delete {
                hook_id,
                redacted_url,
            } => {
                client.delete_repo_webhook(repo, *hook_id).await?;
                report
                    .applied
                    .push(format!("Deleted webhook {redacted_url}"));
            }
        }
    }

    for action in &plan.deploy_key_actions {
        match action {
            DeployKeyAction::Create(desired) => {
                report
                    .applied
                    .push(apply_deploy_key_create(client, repo, desired).await?);
            }
            DeployKeyAction::Replace {
                key_id,
                current_title,
                desired,
            } => {
                let created = apply_deploy_key_create(client, repo, desired).await?;
                client.delete_repo_deploy_key(repo, *key_id).await?;
                report.applied.push(created);
                report
                    .applied
                    .push(format!("Deleted replaced deploy key {current_title}"));
            }
            DeployKeyAction::Delete { key_id, title } => {
                client.delete_repo_deploy_key(repo, *key_id).await?;
                report.applied.push(format!("Deleted deploy key {title}"));
            }
        }
    }

    if let Some(action) = &plan.pages_action {
        match action {
            PagesAction::Create(desired) | PagesAction::Update(desired)
                if !matches!(desired.build_type.as_deref(), Some("workflow")) =>
            {
                if let Some(branch) = desired.source_branch.as_deref() {
                    if !client.branch_exists(repo, branch).await? {
                        report.blocked.push(format!(
                            "GitHub Pages source branch `{branch}` does not exist."
                        ));
                    } else if matches!(action, PagesAction::Create(_)) {
                        client.create_repo_pages(repo, desired).await?;
                        report.applied.push("Created GitHub Pages site".to_owned());
                    } else {
                        client.update_repo_pages(repo, desired).await?;
                        report.applied.push("Updated GitHub Pages site".to_owned());
                    }
                } else {
                    report.blocked.push(
                        "GitHub Pages source branch is required for non-workflow Pages configuration."
                            .to_owned(),
                    );
                }
            }
            PagesAction::Create(desired) => {
                client.create_repo_pages(repo, desired).await?;
                report.applied.push("Created GitHub Pages site".to_owned());
            }
            PagesAction::Update(desired) => {
                client.update_repo_pages(repo, desired).await?;
                report.applied.push("Updated GitHub Pages site".to_owned());
            }
            PagesAction::Delete => {
                client.delete_repo_pages(repo).await?;
                report.applied.push("Deleted GitHub Pages site".to_owned());
            }
        }
    }

    for action in &plan.autolink_actions {
        match action {
            AutolinkAction::Create(desired) => {
                client.create_repo_autolink(repo, desired).await?;
                report
                    .applied
                    .push(format!("Created autolink {}", desired.key_prefix));
            }
            AutolinkAction::Recreate {
                autolink_id,
                desired,
            } => {
                client.delete_repo_autolink(repo, *autolink_id).await?;
                client.create_repo_autolink(repo, desired).await?;
                report
                    .applied
                    .push(format!("Recreated autolink {}", desired.key_prefix));
            }
            AutolinkAction::Delete {
                autolink_id,
                key_prefix,
            } => {
                client.delete_repo_autolink(repo, *autolink_id).await?;
                report
                    .applied
                    .push(format!("Deleted autolink {key_prefix}"));
            }
        }
    }

    Ok(report)
}

pub async fn verify_integrations(
    client: &Client,
    repo: &str,
    desired: &RepositoryIntegrationsCategoryV2,
) -> Result<IntegrationsVerification> {
    let current = collect_integrations(client, repo, desired).await?;
    Ok(verify_integrations_state(&current, desired))
}

pub fn verify_integrations_state(
    current: &IntegrationsCollection,
    desired: &RepositoryIntegrationsCategoryV2,
) -> IntegrationsVerification {
    let mut verification = IntegrationsVerification::default();
    let current_webhooks = current
        .state
        .webhooks
        .iter()
        .map(|hook| (hook.canonical_url.clone(), hook))
        .collect::<BTreeMap<_, _>>();

    if current.state.webhooks_complete {
        for desired_hook in &desired.webhooks {
            let canonical = canonicalize_url(&desired_hook.url);
            match current_webhooks.get(&canonical) {
                None => verification
                    .issues
                    .push(format!("Missing webhook {}", desired_hook.url)),
                Some(current_hook)
                    if normalized_webhook(current_hook)
                        != normalized_webhook_config(desired_hook) =>
                {
                    verification.issues.push(format!(
                        "Webhook {} differs from desired state",
                        desired_hook.url
                    ));
                }
                _ => {}
            }
        }

        if desired.policy.prune {
            let desired_keys = desired
                .webhooks
                .iter()
                .map(|hook| canonicalize_url(&hook.url))
                .collect::<BTreeSet<_>>();
            for current_hook in &current.state.webhooks {
                if !desired_keys.contains(&current_hook.canonical_url) {
                    verification.issues.push(format!(
                        "Unexpected webhook {} is still configured",
                        current_hook.config.url
                    ));
                }
            }
        }
    } else if !desired.webhooks.is_empty() || desired.policy.prune {
        verification.notes.push(
            "Could not fully verify webhooks because webhook collection was incomplete.".to_owned(),
        );
    }

    if current.state.deploy_keys_complete {
        for desired_key in &desired.deploy_keys {
            match match_deploy_key(&current.state, desired_key) {
                None => verification
                    .issues
                    .push(format!("Missing deploy key {}", desired_key.title)),
                Some((_, current_key)) if !deploy_key_equivalent(current_key, desired_key) => {
                    verification.issues.push(format!(
                        "Deploy key {} differs from desired state",
                        desired_key.title
                    ));
                }
                _ => {}
            }
        }

        if desired.policy.prune {
            let expected_ids = desired
                .deploy_keys
                .iter()
                .filter_map(|desired_key| {
                    match_deploy_key(&current.state, desired_key).map(|(id, _)| id)
                })
                .collect::<BTreeSet<_>>();
            for current_key in &current.state.deploy_keys {
                if !expected_ids.contains(&current_key.id) {
                    verification.issues.push(format!(
                        "Unexpected deploy key {} is still configured",
                        current_key.config.title
                    ));
                }
            }
        }
    } else if !desired.deploy_keys.is_empty() || desired.policy.prune {
        verification.notes.push(
            "Could not fully verify deploy keys because deploy-key collection was incomplete."
                .to_owned(),
        );
    }

    if current.state.pages_complete {
        match (&current.state.pages, &desired.pages) {
            (None, Some(_)) => verification
                .issues
                .push("GitHub Pages site is missing".to_owned()),
            (Some(_), None) if desired.policy.prune => verification
                .issues
                .push("GitHub Pages site still exists".to_owned()),
            (Some(current_pages), Some(desired_pages))
                if normalized_pages(&current_pages.config) != normalized_pages(desired_pages) =>
            {
                verification
                    .issues
                    .push("GitHub Pages site differs from desired state".to_owned());
            }
            (Some(current_pages), Some(_)) => {
                if let Some(status) = &current_pages.status
                    && status != "built"
                {
                    verification
                        .notes
                        .push(format!("GitHub Pages status is `{status}`."));
                }
            }
            _ => {}
        }
    } else if desired.pages.is_some() || desired.policy.prune {
        verification.notes.push(
            "Could not fully verify GitHub Pages because Pages collection was incomplete."
                .to_owned(),
        );
    }

    if current.state.autolinks_complete {
        let current_autolinks = current
            .state
            .autolinks
            .iter()
            .map(|autolink| (autolink.config.key_prefix.as_str(), autolink))
            .collect::<BTreeMap<_, _>>();
        for desired_autolink in &desired.autolinks {
            match current_autolinks.get(desired_autolink.key_prefix.as_str()) {
                None => verification
                    .issues
                    .push(format!("Missing autolink {}", desired_autolink.key_prefix)),
                Some(current_autolink)
                    if normalized_autolink(&current_autolink.config)
                        != normalized_autolink(desired_autolink) =>
                {
                    verification.issues.push(format!(
                        "Autolink {} differs from desired state",
                        desired_autolink.key_prefix
                    ));
                }
                _ => {}
            }
        }

        if desired.policy.prune {
            let desired_prefixes = desired
                .autolinks
                .iter()
                .map(|autolink| autolink.key_prefix.as_str())
                .collect::<BTreeSet<_>>();
            for current_autolink in &current.state.autolinks {
                if !desired_prefixes.contains(current_autolink.config.key_prefix.as_str()) {
                    verification.issues.push(format!(
                        "Unexpected autolink {} is still configured",
                        current_autolink.config.key_prefix
                    ));
                }
            }
        }
    } else if !desired.autolinks.is_empty() || desired.policy.prune {
        verification.notes.push(
            "Could not fully verify autolinks because autolink collection was incomplete."
                .to_owned(),
        );
    }

    if !desired.labels.is_empty() {
        verification.notes.push(
            "Labels remain owned by general-settings and are intentionally ignored here."
                .to_owned(),
        );
    }

    verification
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
    collect_selected_repository_reference(
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
    )
}

async fn collect_org_variable_reference(
    client: &Client,
    repo: &str,
    reference: &ReferencedResourceConfig,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<CollectedAccessReference> {
    collect_selected_repository_reference(
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
    )
}

fn collect_selected_repository_reference(
    repo: &str,
    reference: &ReferencedResourceConfig,
    endpoints: (&str, &str),
    metadata: ReadOutcome<Option<OrgScopedResourceMetadata>>,
    repositories: ReadOutcome<Option<Vec<NamedRepository>>>,
    coverage: &mut Vec<CoverageEntry>,
) -> Result<CollectedAccessReference> {
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
            return Ok(CollectedAccessReference {
                resource: reference.clone(),
                present: None,
                associated: None,
                supported: true,
                detail: Some("Referenced organization resource lookup was unavailable.".to_owned()),
            });
        }
    };

    let Some(metadata) = metadata else {
        return Ok(CollectedAccessReference {
            resource: reference.clone(),
            present: Some(false),
            associated: None,
            supported: true,
            detail: Some("Referenced organization resource was not found.".to_owned()),
        });
    };

    if metadata.visibility.as_deref() != Some("selected") {
        return Ok(CollectedAccessReference {
            resource: reference.clone(),
            present: Some(true),
            associated: None,
            supported: false,
            detail: Some(format!(
                "{} visibility is {:?}; selected-repository association is not applicable.",
                reference.name, metadata.visibility
            )),
        });
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
            return Ok(CollectedAccessReference {
                resource: reference.clone(),
                present: Some(true),
                associated: None,
                supported: true,
                detail: Some("Selected-repository association lookup was unavailable.".to_owned()),
            });
        }
    };

    Ok(CollectedAccessReference {
        resource: reference.clone(),
        present: Some(true),
        associated: repositories
            .as_deref()
            .map(|repositories| repositories.iter().any(|entry| entry.name == repo)),
        supported: true,
        detail: None,
    })
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

async fn apply_deploy_key_create(
    client: &Client,
    repo: &str,
    desired: &DeployKeyConfigV2,
) -> Result<String> {
    let replacement_key =
        resolve_required_external_value(desired.replacement_key.as_ref(), "deploy key")
            .map_err(anyhow::Error::msg)?;
    client
        .create_repo_deploy_key(
            repo,
            &desired.title,
            &replacement_key,
            desired.read_only.unwrap_or(true),
        )
        .await?;
    Ok(format!("Created deploy key {}", desired.title))
}

fn deploy_key_equivalent(current: &CollectedDeployKey, desired: &DeployKeyConfigV2) -> bool {
    current.config.title == desired.title
        && current.config.read_only.unwrap_or(true) == desired.read_only.unwrap_or(true)
        && current.config.fingerprint == desired.fingerprint
}

fn match_deploy_key<'a>(
    current: &'a CollectedIntegrationsState,
    desired: &DeployKeyConfigV2,
) -> Option<(u64, &'a CollectedDeployKey)> {
    if let Some(fingerprint) = desired.fingerprint.as_deref()
        && let Some(entry) = current
            .deploy_keys
            .iter()
            .find(|entry| entry.config.fingerprint.as_deref() == Some(fingerprint))
    {
        return Some((entry.id, entry));
    }

    current
        .deploy_keys
        .iter()
        .find(|entry| entry.config.title == desired.title)
        .map(|entry| (entry.id, entry))
}

fn deploy_key_block_reason(desired: &DeployKeyConfigV2) -> Option<String> {
    match desired.replacement_key.as_ref() {
        None => Some("replacement_key is required to create or rotate deploy keys.".to_owned()),
        Some(ExternalValueReference::Manual { .. }) => Some(
            "replacement_key is manual-only; Ward will not guess deploy key material.".to_owned(),
        ),
        Some(ExternalValueReference::Env { key }) if env::var(key).is_err() => Some(format!(
            "replacement_key environment variable {key} is not set."
        )),
        _ => None,
    }
}

fn webhook_create_block_reason(desired: &WebhookConfigV2) -> Option<String> {
    resolve_required_url(desired).err().or_else(|| {
        resolve_required_external_value(desired.secret.as_ref(), "webhook secret").err()
    })
}

fn webhook_update_block_reason(
    current: &CollectedWebhook,
    desired: &WebhookConfigV2,
) -> Option<String> {
    let desired_normalized = normalized_webhook_config(desired);
    if current.canonical_url != desired_normalized.canonical_url {
        resolve_required_url(desired).err()
    } else {
        None
    }
}

fn normalized_webhook(current: &CollectedWebhook) -> NormalizedWebhook {
    normalized_webhook_config(&current.config)
}

fn normalized_webhook_config(webhook: &WebhookConfigV2) -> NormalizedWebhook {
    NormalizedWebhook {
        canonical_url: canonicalize_url(&webhook.url),
        active: webhook.active.unwrap_or(true),
        events: normalize_events(&webhook.events),
        content_type: webhook.content_type.clone(),
        insecure_ssl: webhook.insecure_ssl,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedWebhook {
    canonical_url: String,
    active: bool,
    events: Vec<String>,
    content_type: Option<String>,
    insecure_ssl: Option<bool>,
}

fn normalized_pages(pages: &PagesConfigV2) -> NormalizedPages {
    let build_type = pages.build_type.clone();
    let workflow = matches!(build_type.as_deref(), Some("workflow"));
    NormalizedPages {
        build_type,
        source_branch: if workflow {
            None
        } else {
            pages.source_branch.clone()
        },
        source_path: if workflow {
            None
        } else {
            pages.source_path.clone()
        },
        cname: pages.cname.clone(),
        https_enforced: pages.https_enforced,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedPages {
    build_type: Option<String>,
    source_branch: Option<String>,
    source_path: Option<String>,
    cname: Option<String>,
    https_enforced: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedAutolink {
    key_prefix: String,
    url_template: String,
    is_alphanumeric: bool,
}

fn normalized_autolink(autolink: &AutolinkConfigV2) -> NormalizedAutolink {
    NormalizedAutolink {
        key_prefix: autolink.key_prefix.clone(),
        url_template: autolink.url_template.clone(),
        is_alphanumeric: autolink.is_alphanumeric.unwrap_or(true),
    }
}

fn normalize_events(events: &[String]) -> Vec<String> {
    let mut events = events.to_vec();
    events.sort();
    events.dedup();
    events
}

fn actor_login(actor: &ActorReference) -> Option<&str> {
    match actor {
        ActorReference::User { login } => Some(login.as_str()),
        _ => None,
    }
}

fn resolve_optional_external_value(
    reference: Option<&ExternalValueReference>,
) -> Result<Option<String>, String> {
    match reference {
        None => Ok(None),
        Some(ExternalValueReference::Env { key }) => env::var(key)
            .map(Some)
            .map_err(|_| format!("environment variable `{key}` is not set")),
        Some(ExternalValueReference::Manual { hint }) => Err(match hint {
            Some(hint) => format!("value must be provided manually ({hint})"),
            None => "value must be provided manually".to_owned(),
        }),
    }
}

fn resolve_required_external_value(
    reference: Option<&ExternalValueReference>,
    label: &str,
) -> Result<String, String> {
    match reference {
        Some(ExternalValueReference::Env { key }) => {
            env::var(key).map_err(|_| format!("{label} environment variable {key} is not set"))
        }
        Some(ExternalValueReference::Manual { .. }) => Err(format!(
            "{label} is manual-only and cannot be applied automatically"
        )),
        None => Err(format!("{label} is missing required external value")),
    }
}

fn resolve_required_url(webhook: &WebhookConfigV2) -> Result<String, String> {
    if let Some(reference) = webhook.url_from.as_ref() {
        return resolve_required_external_value(Some(reference), "webhook URL");
    }
    if let Ok(parsed) = reqwest::Url::parse(&webhook.url)
        && parsed.username() == "***"
    {
        return Err(
            "credentialed webhook URL is redacted; provide `url_from` with the real URL before applying."
                .to_owned(),
        );
    }
    if let Some(key) = env_placeholder_key(&webhook.url) {
        env::var(key).map_err(|_| format!("webhook URL environment variable {key} is not set"))
    } else {
        Ok(webhook.url.clone())
    }
}

pub fn canonicalize_url(url: &str) -> String {
    if env_placeholder_key(url).is_some() {
        return url.to_owned();
    }
    match reqwest::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            if (parsed.scheme() == "http" && parsed.port() == Some(80))
                || (parsed.scheme() == "https" && parsed.port() == Some(443))
            {
                let _ = parsed.set_port(None);
            }
            let mut canonical = parsed.to_string();
            if canonical.ends_with('/') && parsed.query().is_none() && parsed.fragment().is_none() {
                canonical.pop();
            }
            canonical
        }
        Err(_) => url.to_owned(),
    }
}

fn env_placeholder_key(value: &str) -> Option<&str> {
    value
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
        .filter(|key| !key.is_empty())
}

fn imported_webhook_identity(url: &str) -> (String, Option<ExternalValueReference>) {
    match reqwest::Url::parse(url) {
        Ok(parsed) if !parsed.username().is_empty() || parsed.password().is_some() => {
            let key = credentialed_webhook_url_env_key(&parsed);
            (
                redact_credentialed_url(url),
                Some(ExternalValueReference::Env { key }),
            )
        }
        Ok(_) => (canonicalize_url(url), None),
        Err(_) => (url.to_owned(), None),
    }
}

fn redact_credentialed_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut parsed) if !parsed.username().is_empty() || parsed.password().is_some() => {
            let _ = parsed.set_username("***");
            let _ = parsed.set_password(None);
            parsed.to_string()
        }
        Ok(_) => canonicalize_url(url),
        Err(_) => url.to_owned(),
    }
}

fn credentialed_webhook_url_env_key(parsed: &reqwest::Url) -> String {
    let mut seed = format!("{}{}", parsed.host_str().unwrap_or("hook"), parsed.path());
    if let Some(query) = parsed.query() {
        seed.push('_');
        seed.push_str(query);
    }
    let suffix = seed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_owned();
    format!("{WEBHOOK_URL_ENV_PREFIX}{suffix}")
}

fn derive_access_references(
    desired: &RepositoryAccessCategoryV2,
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

fn is_custom_repository_permission(permission: &str) -> bool {
    !BUILTIN_REPOSITORY_PERMISSIONS.contains(&permission)
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

fn format_issue(issue: &ReconcileIssue) -> String {
    format!("{}: {}", issue.scope, issue.message)
}

fn record_read_outcome<T>(
    coverage: &mut Vec<CoverageEntry>,
    category: ManifestCategoryName,
    endpoint: &str,
    outcome: ReadOutcome<T>,
) -> Option<T> {
    match outcome {
        ReadOutcome::Available(value) => Some(value),
        ReadOutcome::NotApplicable(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::NotApplicable,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        ReadOutcome::PermissionDenied(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::PermissionDenied,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        ReadOutcome::Unavailable(reason) => {
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

fn apply_access_policy_gates(
    desired: &RepositoryAccessCategoryV2,
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

fn has_blocker_for_scope(issues: &[ReconcileIssue], scope: &str) -> bool {
    issues
        .iter()
        .any(|issue| issue.severity == IssueSeverity::Blocker && issue.scope.starts_with(scope))
}

fn validate_pages_desired(desired: Option<&PagesConfigV2>) -> Vec<ReconcileIssue> {
    let Some(desired) = desired else {
        return Vec::new();
    };
    let mut issues = Vec::new();
    let workflow = matches!(desired.build_type.as_deref(), Some("workflow"));
    if workflow {
        if desired.source_branch.is_some() || desired.source_path.is_some() {
            issues.push(ReconcileIssue {
                scope: "integrations.pages".to_owned(),
                severity: IssueSeverity::Blocker,
                message:
                    "Workflow-based Pages configuration must not set source_branch or source_path."
                        .to_owned(),
            });
        }
        return issues;
    }

    if desired.source_branch.is_some() ^ desired.source_path.is_some() {
        issues.push(ReconcileIssue {
            scope: "integrations.pages".to_owned(),
            severity: IssueSeverity::Blocker,
            message:
                "Non-workflow Pages configuration must set both source_branch and source_path."
                    .to_owned(),
        });
    }

    issues
}

fn apply_integrations_policy_gates(
    desired: &RepositoryIntegrationsCategoryV2,
    webhook_actions: &mut Vec<WebhookAction>,
    deploy_key_actions: &mut Vec<DeployKeyAction>,
    pages_action: &mut Option<PagesAction>,
    autolink_actions: &mut Vec<AutolinkAction>,
    issues: &mut Vec<ReconcileIssue>,
) {
    if desired.policy.disposition != ManagementDisposition::Managed {
        if !webhook_actions.is_empty()
            || !deploy_key_actions.is_empty()
            || pages_action.is_some()
            || !autolink_actions.is_empty()
        {
            issues.push(ReconcileIssue {
                scope: "integrations".to_owned(),
                severity: IssueSeverity::Warning,
                message: "Integration changes observed but integrations category is not managed."
                    .to_owned(),
            });
        }
        webhook_actions.clear();
        deploy_key_actions.clear();
        *pages_action = None;
        autolink_actions.clear();
        return;
    }

    if !desired.policy.sensitive {
        if !webhook_actions.is_empty()
            || !deploy_key_actions.is_empty()
            || pages_action.is_some()
            || desired.policy.prune
        {
            issues.push(ReconcileIssue {
                scope: "integrations".to_owned(),
                severity: IssueSeverity::Blocker,
                message: "Webhook, deploy-key, Pages, and prune mutations require `policy.sensitive: true`.".to_owned(),
            });
        }
        webhook_actions.clear();
        deploy_key_actions.clear();
        *pages_action = None;
        if desired.policy.prune {
            autolink_actions.retain(|action| !matches!(action, AutolinkAction::Delete { .. }));
        }
    }
}
