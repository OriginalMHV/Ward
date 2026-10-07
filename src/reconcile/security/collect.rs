//! Security state collection.

use std::collections::HashMap;

use anyhow::Result;

use crate::config::manifest::{
    ActorReference, CategoryPolicy, CodeqlDefaultSetupConfig, ManifestCategoryName,
    ReferencedResourceConfig, ReferencedResourceType, SecurityCategory, SecurityReviewerConfig,
    SecurityReviewerOptionsConfig,
};
use crate::github::Client;
use crate::github::security::{
    CodeqlDefaultSetupState, RepositoryCodeSecurityConfiguration, RepositorySecurityBaseline,
};
use crate::reconcile::common::actors::repository_role_lookup;
use crate::reconcile::common::coverage::{
    collected_entry, not_applicable_entry, permission_denied_entry, unavailable_entry,
};
use crate::reconcile::common::rules_issue::ReconcileIssue;
use crate::reconcile::common::rules_issue::warning_issue;

use super::*;

pub async fn collect_security_category(
    client: &Client,
    repo: &str,
    category: Option<&SecurityCategory>,
) -> Result<SecurityCollection> {
    let baseline = client.get_repository_security_baseline(repo).await?;
    collect_security_category_with_baseline(client, repo, baseline, category).await
}

/// Collect security state from an already fetched `GET /repos/{repo}` response.
pub async fn collect_security_category_with_baseline(
    client: &Client,
    repo: &str,
    baseline: RepositorySecurityBaseline,
    category: Option<&SecurityCategory>,
) -> Result<SecurityCollection> {
    let RepositorySecurityBaseline {
        id: repository_id,
        security_and_analysis,
    } = baseline;
    let mut issues = Vec::new();
    let mut coverage = vec![collected_entry(
        ManifestCategoryName::Security,
        "GET /repos/{owner}/{repo}",
    )];
    let analysis = security_and_analysis.unwrap_or_default();

    let dependabot_alerts = match client.read_dependabot_alerts_state(repo).await? {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
                reason,
            ));
            None
        }
    };

    let dependabot_security_updates_endpoint =
        match client.read_dependabot_security_updates_state(repo).await? {
            crate::github::actions::ReadOutcome::Available(value) => {
                coverage.push(collected_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                ));
                Some(value)
            }
            crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
                coverage.push(permission_denied_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                    reason,
                ));
                None
            }
            crate::github::actions::ReadOutcome::NotApplicable(reason) => {
                coverage.push(not_applicable_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                    reason,
                ));
                None
            }
            crate::github::actions::ReadOutcome::Unavailable(reason) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                    reason,
                ));
                None
            }
        };

    let private_vulnerability_reporting = match client
        .read_private_vulnerability_reporting_status(repo)
        .await?
    {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
                reason,
            ));
            None
        }
    };

    let codeql_default_setup = match client.read_codeql_default_setup(repo).await? {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
                reason,
            ));
            None
        }
    };

    let attached_configuration = match client
        .read_repository_code_security_configuration(repo)
        .await?
    {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
                reason,
            ));
            None
        }
    };

    let available_configurations = match client.list_code_security_configurations().await {
        Ok(mut value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /orgs/{org}/code-security/configurations",
            ));
            value.sort_by(|left, right| left.name.cmp(&right.name));
            value
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /orgs/{org}/code-security/configurations",
                format!("{error:#}"),
            ));
            Vec::new()
        }
    };

    let configuration_reference = attached_configuration
        .as_ref()
        .map(configuration_reference_from_attachment);

    let mut team_ids_by_slug = HashMap::new();
    let mut repository_role_ids_by_name = HashMap::new();
    if analysis
        .secret_scanning_delegated_alert_dismissal_options
        .is_some()
        || analysis.secret_scanning_delegated_bypass_options.is_some()
    {
        match client.list_org_teams().await {
            Ok(teams) => {
                for team in teams {
                    team_ids_by_slug.insert(team.slug, team.id);
                }
            }
            Err(error) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Security,
                    "GET /orgs/{org}/teams",
                    format!("{error:#}"),
                ));
                issues.push(warning_issue(
                    Some(repo.to_owned()),
                    "security-reviewer-team-resolution",
                    format!("Could not resolve delegated reviewer team IDs for {repo}: {error}"),
                ));
            }
        }
        match client.list_ruleset_custom_repository_roles().await {
            Ok(roles) => {
                for (id, name) in repository_role_lookup(&roles) {
                    repository_role_ids_by_name.insert(name, id);
                }
            }
            Err(error) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Security,
                    "GET /orgs/{org}/custom-repository-roles",
                    format!("{error:#}"),
                ));
                issues.push(warning_issue(
                    Some(repo.to_owned()),
                    "security-reviewer-role-resolution",
                    format!("Could not resolve delegated reviewer role IDs for {repo}: {error}"),
                ));
            }
        }
    }

    let delegated_alert_dismissal_options = analysis
        .secret_scanning_delegated_alert_dismissal_options
        .as_ref()
        .map(|options| {
            reviewer_options_from_api(
                options,
                &team_ids_by_slug,
                &repository_role_ids_by_name,
                repo,
                &mut issues,
            )
        })
        .transpose()?;
    let delegated_bypass_options = analysis
        .secret_scanning_delegated_bypass_options
        .as_ref()
        .map(|options| {
            reviewer_options_from_api(
                options,
                &team_ids_by_slug,
                &repository_role_ids_by_name,
                repo,
                &mut issues,
            )
        })
        .transpose()?;

    let delegated_alert_dismissal_reviewers = delegated_alert_dismissal_options
        .as_ref()
        .map(|options| {
            options
                .reviewers
                .iter()
                .map(|reviewer| reviewer.actor.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let delegated_bypass_reviewers = delegated_bypass_options
        .as_ref()
        .map(|options| {
            options
                .reviewers
                .iter()
                .map(|reviewer| reviewer.actor.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let references = configuration_reference.iter().cloned().collect::<Vec<_>>();

    let policy = category
        .map(|value| value.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    Ok(SecurityCollection {
        repository_id,
        category: SecurityCategory {
            policy,
            advanced_security: bool_field(analysis.advanced_security.as_ref()),
            code_security: bool_field(analysis.code_security.as_ref()),
            dependabot_alerts,
            dependabot_security_updates: dependabot_security_updates_endpoint
                .or_else(|| bool_field(analysis.dependabot_security_updates.as_ref())),
            secret_scanning: bool_field(analysis.secret_scanning.as_ref()),
            secret_scanning_push_protection: bool_field(
                analysis.secret_scanning_push_protection.as_ref(),
            ),
            secret_scanning_validity_checks: bool_field(
                analysis.secret_scanning_validity_checks.as_ref(),
            ),
            secret_scanning_non_provider_patterns: bool_field(
                analysis.secret_scanning_non_provider_patterns.as_ref(),
            ),
            secret_scanning_ai_detection: bool_field(
                analysis.secret_scanning_ai_detection.as_ref(),
            ),
            secret_scanning_delegated_alert_dismissal: bool_field(
                analysis.secret_scanning_delegated_alert_dismissal.as_ref(),
            ),
            secret_scanning_delegated_bypass: bool_field(
                analysis.secret_scanning_delegated_bypass.as_ref(),
            ),
            secret_scanning_delegated_alert_dismissal_options: delegated_alert_dismissal_options,
            secret_scanning_delegated_bypass_options: delegated_bypass_options,
            private_vulnerability_reporting,
            codeql_default_setup: codeql_default_setup.as_ref().map(codeql_state_to_manifest),
            configuration_reference,
            delegated_alert_dismissal_reviewers,
            delegated_bypass_reviewers,
            references,
        },
        analysis,
        private_vulnerability_reporting,
        codeql_default_setup,
        attached_configuration,
        available_configurations,
        team_ids_by_slug,
        repository_role_ids_by_name,
        coverage,
        issues,
    })
}

pub(super) fn codeql_state_to_manifest(
    state: &CodeqlDefaultSetupState,
) -> CodeqlDefaultSetupConfig {
    CodeqlDefaultSetupConfig {
        state: state.state.clone(),
        languages: state.languages.clone(),
        query_suite: state.query_suite.clone(),
        runner_type: state.runner_type.clone(),
        runner_label: state.runner_label.clone(),
        threat_model: state.threat_model.clone(),
    }
}

fn configuration_reference_from_attachment(
    attachment: &RepositoryCodeSecurityConfiguration,
) -> ReferencedResourceConfig {
    ReferencedResourceConfig {
        resource_type: ReferencedResourceType::CodeSecurityConfiguration,
        name: attachment.configuration.name.clone(),
    }
}

fn reviewer_options_from_api(
    options: &crate::github::security::DelegatedBypassOptions,
    team_ids_by_slug: &HashMap<String, u64>,
    repository_role_ids_by_name: &HashMap<String, u64>,
    repo: &str,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<SecurityReviewerOptionsConfig> {
    let team_slugs_by_id = team_ids_by_slug
        .iter()
        .map(|(slug, id)| (*id, slug.clone()))
        .collect::<HashMap<_, _>>();
    let role_names_by_id = repository_role_ids_by_name
        .iter()
        .map(|(name, id)| (*id, name.clone()))
        .collect::<HashMap<_, _>>();

    let reviewers = options
        .reviewers
        .iter()
        .map(|reviewer| {
            let actor = match reviewer.reviewer_type.as_str() {
                "Team" | "TEAM" => team_slugs_by_id
                    .get(&reviewer.reviewer_id)
                    .cloned()
                    .map(|slug| ActorReference::Team { slug })
                    .unwrap_or_else(|| {
                        issues.push(warning_issue(
                            Some(repo.to_owned()),
                            "security-reviewer-team-unresolved",
                            format!(
                                "Could not resolve delegated reviewer team id {} for {repo}; preserving it as unresolved.",
                                reviewer.reviewer_id
                            ),
                        ));
                        ActorReference::Unresolved {
                            actor_type: reviewer.reviewer_type.clone(),
                            actor_id: Some(reviewer.reviewer_id),
                        }
                    }),
                "RepositoryRole" | "ROLE" => role_names_by_id
                    .get(&reviewer.reviewer_id)
                    .cloned()
                    .map(|name| ActorReference::Role { name })
                    .unwrap_or_else(|| {
                        issues.push(warning_issue(
                            Some(repo.to_owned()),
                            "security-reviewer-role-unresolved",
                            format!(
                                "Could not resolve delegated reviewer repository role id {} for {repo}; preserving it as unresolved.",
                                reviewer.reviewer_id
                            ),
                        ));
                        ActorReference::Unresolved {
                            actor_type: reviewer.reviewer_type.clone(),
                            actor_id: Some(reviewer.reviewer_id),
                        }
                    }),
                _ => {
                    issues.push(warning_issue(
                        Some(repo.to_owned()),
                        "security-reviewer-actor-unsupported",
                        format!(
                            "Delegated reviewer type {} on {repo} is not supported for stable reconciliation; preserving it as unresolved.",
                            reviewer.reviewer_type
                        ),
                    ));
                    ActorReference::Unresolved {
                        actor_type: reviewer.reviewer_type.clone(),
                        actor_id: Some(reviewer.reviewer_id),
                    }
                }
            };
            Ok(SecurityReviewerConfig {
                actor,
                mode: reviewer.mode.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(SecurityReviewerOptionsConfig { reviewers })
}

fn bool_field(value: Option<&crate::github::security::SecurityFeatureStatus>) -> Option<bool> {
    value.map(|status| status.status == "enabled")
}
