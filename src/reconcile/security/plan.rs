//! Security plan computation.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::config::manifest::{
    ActorReference, CodeqlDefaultSetupConfig, ManagementDisposition, SecurityCategory,
    SecurityReviewerConfig, SecurityReviewerOptionsConfig,
};
use crate::github::security::CodeqlDefaultSetupState;
use crate::reconcile::common::rules_issue::{blocker_issue, warning_issue};

use super::collect::*;
use super::*;

const SECURITY_ATTACHED_CONFIGURATION_PATH: &str = "categories.security.configuration_reference";

pub fn plan_security_category(
    desired: &SecurityCategory,
    actual: &SecurityCollection,
) -> Result<SecurityPlan> {
    let mut issues = actual.issues.clone();
    let mut patch = serde_json::Map::new();
    let mut dependabot_alerts = None;
    let mut dependabot_security_updates = None;
    let mut private_vulnerability_reporting = None;
    let mut codeql_default_setup = None;
    let mut attach_configuration_id = None;
    let mut detach_configuration = false;

    if desired.policy.disposition != ManagementDisposition::Managed {
        return Ok(SecurityPlan {
            repository_id: actual.repository_id,
            patch_security_and_analysis: None,
            dependabot_alerts,
            dependabot_security_updates,
            private_vulnerability_reporting,
            codeql_default_setup,
            attach_configuration_id,
            detach_configuration,
            issues,
        });
    }

    let attached_configuration_present = actual.attached_configuration.is_some();
    let detaching_existing_configuration = attached_configuration_present
        && desired.configuration_reference.is_none()
        && desired.policy.prune;

    if desired.configuration_reference.is_some() || detaching_existing_configuration {
        if let Some(reference) = &desired.configuration_reference {
            if let Some(configuration) =
                actual
                    .available_configurations
                    .iter()
                    .find(|configuration| {
                        configuration.name == reference.name
                            && matches!(
                                configuration.target_type.as_str(),
                                "organization" | "global" | ""
                            )
                    })
            {
                if actual
                    .attached_configuration
                    .as_ref()
                    .map(|attached| attached.configuration.id)
                    != Some(configuration.id)
                {
                    attach_configuration_id = Some(configuration.id);
                }
            } else {
                issues.push(blocker_issue(
                    Some(reference.name.clone()),
                    "security-missing-configuration",
                    format!(
                        "Code security configuration {} is not available for organization {}",
                        reference.name,
                        actual
                            .attached_configuration
                            .as_ref()
                            .map(|_| "")
                            .unwrap_or(clientless_org_hint())
                    ),
                ));
            }
        } else if detaching_existing_configuration {
            detach_configuration = true;
        }

        if (attach_configuration_id.is_some() || detach_configuration) && !desired.policy.sensitive
        {
            issues.push(blocker_issue(
                Some(SECURITY_ATTACHED_CONFIGURATION_PATH.to_owned()),
                "security-sensitive-gate",
                "Changing code security configuration attachments requires policy.sensitive = true"
                    .to_owned(),
            ));
        }

        if desired.private_vulnerability_reporting.is_some()
            || desired.codeql_default_setup.is_some()
            || !desired.delegated_alert_dismissal_reviewers.is_empty()
            || !desired.delegated_bypass_reviewers.is_empty()
        {
            issues.push(warning_issue(
                Some(SECURITY_ATTACHED_CONFIGURATION_PATH.to_owned()),
                "security-attached-configuration-precedence",
                "Attached code security configurations take precedence over per-repository security toggles; direct settings will be ignored in this plan.".to_owned(),
            ));
        }

        return Ok(SecurityPlan {
            repository_id: actual.repository_id,
            patch_security_and_analysis: None,
            dependabot_alerts,
            dependabot_security_updates,
            private_vulnerability_reporting,
            codeql_default_setup,
            attach_configuration_id,
            detach_configuration,
            issues,
        });
    }

    if attached_configuration_present {
        issues.push(warning_issue(
            Some(SECURITY_ATTACHED_CONFIGURATION_PATH.to_owned()),
            "security-attached-configuration-precedence",
            "An attached code security configuration currently controls this repository; direct security changes are suppressed until the attachment is pruned.".to_owned(),
        ));
    }

    if !attached_configuration_present {
        for (path, desired_value, actual_value) in [
            (
                "advanced_security",
                desired.advanced_security,
                actual.category.advanced_security,
            ),
            (
                "code_security",
                desired.code_security,
                actual.category.code_security,
            ),
            (
                "secret_scanning",
                desired.secret_scanning,
                actual.category.secret_scanning,
            ),
            (
                "secret_scanning_push_protection",
                desired.secret_scanning_push_protection,
                actual.category.secret_scanning_push_protection,
            ),
            (
                "secret_scanning_ai_detection",
                desired.secret_scanning_ai_detection,
                actual.category.secret_scanning_ai_detection,
            ),
            (
                "secret_scanning_non_provider_patterns",
                desired.secret_scanning_non_provider_patterns,
                actual.category.secret_scanning_non_provider_patterns,
            ),
            (
                "secret_scanning_delegated_alert_dismissal",
                desired.secret_scanning_delegated_alert_dismissal,
                actual.category.secret_scanning_delegated_alert_dismissal,
            ),
            (
                "secret_scanning_delegated_bypass",
                desired.secret_scanning_delegated_bypass,
                actual.category.secret_scanning_delegated_bypass,
            ),
        ] {
            if let Some(desired_value) = desired_value
                && actual_value != Some(desired_value)
            {
                patch.insert(path.to_owned(), status_object(desired_value));
            }
        }

        if desired.secret_scanning_validity_checks.is_some()
            && desired.secret_scanning_validity_checks
                != actual.category.secret_scanning_validity_checks
        {
            issues.push(blocker_issue(
                Some("secret_scanning_validity_checks".to_owned()),
                "security-unsupported-validity-checks",
                "The current official repository security_and_analysis API does not expose secret_scanning_validity_checks for direct repository reconciliation.".to_owned(),
            ));
        }

        let desired_alert_dismissal_options = desired
            .secret_scanning_delegated_alert_dismissal_options
            .clone()
            .or_else(|| {
                (!desired.delegated_alert_dismissal_reviewers.is_empty()).then(|| {
                    SecurityReviewerOptionsConfig {
                        reviewers: desired
                            .delegated_alert_dismissal_reviewers
                            .iter()
                            .cloned()
                            .map(|actor| SecurityReviewerConfig { actor, mode: None })
                            .collect(),
                    }
                })
            });
        if desired_alert_dismissal_options.is_some()
            && desired_alert_dismissal_options
                != actual
                    .category
                    .secret_scanning_delegated_alert_dismissal_options
        {
            issues.push(blocker_issue(
                Some("secret_scanning_delegated_alert_dismissal_options".to_owned()),
                "security-unsupported-delegated-alert-dismissal-options",
                "The current official repository security_and_analysis API does not expose delegated alert-dismissal reviewer options for direct repository reconciliation.".to_owned(),
            ));
        }

        let desired_bypass_options = desired
            .secret_scanning_delegated_bypass_options
            .clone()
            .or_else(|| {
                (!desired.delegated_bypass_reviewers.is_empty()).then(|| {
                    SecurityReviewerOptionsConfig {
                        reviewers: desired
                            .delegated_bypass_reviewers
                            .iter()
                            .cloned()
                            .map(|actor| SecurityReviewerConfig { actor, mode: None })
                            .collect(),
                    }
                })
            });
        if desired_bypass_options != actual.category.secret_scanning_delegated_bypass_options {
            if let Some(options) = &desired_bypass_options {
                patch.insert(
                    "secret_scanning_delegated_bypass".to_owned(),
                    status_object(!options.reviewers.is_empty()),
                );
                patch.insert(
                    "secret_scanning_delegated_bypass_options".to_owned(),
                    security_reviewer_options_to_api_json(
                        options,
                        &actual.team_ids_by_slug,
                        &actual.repository_role_ids_by_name,
                    )?,
                );
            } else if desired.secret_scanning_delegated_bypass == Some(false)
                || actual
                    .category
                    .secret_scanning_delegated_bypass_options
                    .is_some()
            {
                patch.insert(
                    "secret_scanning_delegated_bypass".to_owned(),
                    status_object(false),
                );
                patch.insert(
                    "secret_scanning_delegated_bypass_options".to_owned(),
                    serde_json::json!({ "reviewers": [] }),
                );
            }
        }

        if let Some(value) = desired.dependabot_alerts
            && actual.category.dependabot_alerts != Some(value)
        {
            dependabot_alerts = Some(value);
        }

        if let Some(value) = desired.dependabot_security_updates
            && actual.category.dependabot_security_updates != Some(value)
        {
            dependabot_security_updates = Some(value);
        }

        if let Some(value) = desired.private_vulnerability_reporting
            && actual.private_vulnerability_reporting != Some(value)
        {
            private_vulnerability_reporting = Some(value);
        }

        if let Some(codeql) = &desired.codeql_default_setup {
            let desired_codeql = codeql_manifest_to_state(codeql);
            if actual
                .codeql_default_setup
                .as_ref()
                .map(codeql_state_to_manifest)
                != Some(codeql.clone())
            {
                codeql_default_setup = Some(desired_codeql);
            }
        }
    }

    let has_requested_change = !patch.is_empty()
        || dependabot_alerts.is_some()
        || dependabot_security_updates.is_some()
        || private_vulnerability_reporting.is_some()
        || codeql_default_setup.is_some()
        || attach_configuration_id.is_some()
        || detach_configuration;
    if has_requested_change && !desired.policy.sensitive {
        issues.push(blocker_issue(
            Some("categories.security.policy.sensitive".to_owned()),
            "security-sensitive-gate",
            "Managing repository security settings requires policy.sensitive = true".to_owned(),
        ));
    }

    Ok(SecurityPlan {
        repository_id: actual.repository_id,
        patch_security_and_analysis: (!patch.is_empty())
            .then_some(serde_json::Value::Object(patch)),
        dependabot_alerts,
        dependabot_security_updates,
        private_vulnerability_reporting,
        codeql_default_setup,
        attach_configuration_id,
        detach_configuration,
        issues,
    })
}

fn status_object(enabled: bool) -> serde_json::Value {
    serde_json::json!({
        "status": if enabled { "enabled" } else { "disabled" },
    })
}

fn codeql_manifest_to_state(config: &CodeqlDefaultSetupConfig) -> CodeqlDefaultSetupState {
    CodeqlDefaultSetupState {
        state: config.state.clone(),
        languages: config.languages.clone(),
        query_suite: config.query_suite.clone(),
        runner_type: config.runner_type.clone(),
        runner_label: config.runner_label.clone(),
        threat_model: config.threat_model.clone(),
        run_id: None,
    }
}

fn clientless_org_hint() -> &'static str {
    "the configured organization"
}

fn security_reviewer_options_to_api_json(
    options: &SecurityReviewerOptionsConfig,
    team_ids_by_slug: &HashMap<String, u64>,
    repository_role_ids_by_name: &HashMap<String, u64>,
) -> Result<serde_json::Value> {
    let reviewers = options
        .reviewers
        .iter()
        .map(|reviewer| {
            let (reviewer_type, reviewer_id) = match &reviewer.actor {
                ActorReference::Team { slug } => (
                    "TEAM",
                    *team_ids_by_slug
                        .get(slug)
                        .with_context(|| format!("Unknown delegated reviewer team slug {slug}"))?,
                ),
                ActorReference::Role { name } => (
                    "ROLE",
                    *repository_role_ids_by_name.get(name).with_context(|| {
                        format!("Unknown delegated reviewer repository role {name}")
                    })?,
                ),
                ActorReference::Unresolved {
                    actor_type,
                    actor_id: Some(actor_id),
                } => anyhow::bail!(
                    "Cannot apply unresolved delegated reviewer {}:{}",
                    actor_type,
                    actor_id
                ),
                ActorReference::Unresolved {
                    actor_type,
                    actor_id: None,
                } => anyhow::bail!("Cannot apply unresolved delegated reviewer {}", actor_type),
                other => anyhow::bail!(
                    "Unsupported delegated reviewer actor {:?}; only team and repository role reviewers are supported",
                    other
                ),
            };

            let mut value = serde_json::json!({
                "reviewer_type": reviewer_type,
                "reviewer_id": reviewer_id,
            });
            if let Some(mode) = &reviewer.mode {
                value["mode"] =
                    serde_json::Value::String(canonical_security_reviewer_mode(mode)?);
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(serde_json::json!({ "reviewers": reviewers }))
}

fn canonical_security_reviewer_mode(mode: &str) -> Result<String> {
    if mode.eq_ignore_ascii_case("always") {
        Ok("ALWAYS".to_owned())
    } else if mode.eq_ignore_ascii_case("exempt") {
        Ok("EXEMPT".to_owned())
    } else {
        anyhow::bail!("Unsupported delegated reviewer mode {mode}; expected ALWAYS or EXEMPT")
    }
}
