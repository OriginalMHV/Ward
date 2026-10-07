use super::normalize::{
    canonicalize_url, env_placeholder_key, normalized_autolink, normalized_pages,
    normalized_webhook, normalized_webhook_config,
};
use super::{
    AutolinkAction, CollectedDeployKey, CollectedIntegrationsState, CollectedWebhook,
    DeployKeyAction, IntegrationsCollection, IntegrationsPlan, PagesAction, WebhookAction,
};
use crate::config::manifest::{
    DeployKeyConfig, ExternalValueReference, ManagementDisposition, PagesConfig,
    RepositoryIntegrationsCategory, WebhookConfig,
};
use crate::reconcile::common::issue::{IssueSeverity, ReconcileIssue};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::env;

pub fn plan_integrations(
    current: &IntegrationsCollection,
    desired: &RepositoryIntegrationsCategory,
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

pub(super) fn deploy_key_equivalent(
    current: &CollectedDeployKey,
    desired: &DeployKeyConfig,
) -> bool {
    current.config.title == desired.title
        && current.config.read_only.unwrap_or(true) == desired.read_only.unwrap_or(true)
        && current.config.fingerprint == desired.fingerprint
}

pub(super) fn match_deploy_key<'a>(
    current: &'a CollectedIntegrationsState,
    desired: &DeployKeyConfig,
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

fn deploy_key_block_reason(desired: &DeployKeyConfig) -> Option<String> {
    match desired.replacement_key.as_ref() {
        None => Some("replacement_key is required to create or rotate deploy keys.".to_owned()),
        Some(ExternalValueReference::Manual { .. }) => Some(
            "replacement_key is manual-only; Ward will not guess deploy key material.".to_owned(),
        ),
        Some(ExternalValueReference::Env { key }) if read_env(key).is_err() => Some(format!(
            "replacement_key environment variable {key} is not set."
        )),
        _ => None,
    }
}

fn webhook_create_block_reason(desired: &WebhookConfig) -> Option<String> {
    resolve_required_url(desired).err().or_else(|| {
        resolve_required_external_value(desired.secret.as_ref(), "webhook secret").err()
    })
}

fn webhook_update_block_reason(
    current: &CollectedWebhook,
    desired: &WebhookConfig,
) -> Option<String> {
    let desired_normalized = normalized_webhook_config(desired);
    if current.canonical_url != desired_normalized.canonical_url {
        resolve_required_url(desired).err()
    } else {
        None
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "production entry point for env-backed external values"
)]
pub(super) fn read_env(key: &str) -> Result<String, env::VarError> {
    env::var(key)
}

pub(super) fn resolve_required_external_value(
    reference: Option<&ExternalValueReference>,
    label: &str,
) -> Result<String, String> {
    match reference {
        Some(ExternalValueReference::Env { key }) => {
            read_env(key).map_err(|_| format!("{label} environment variable {key} is not set"))
        }
        Some(ExternalValueReference::Manual { .. }) => Err(format!(
            "{label} is manual-only and cannot be applied automatically"
        )),
        None => Err(format!("{label} is missing required external value")),
    }
}

pub(super) fn resolve_required_url(webhook: &WebhookConfig) -> Result<String, String> {
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
        read_env(key).map_err(|_| format!("webhook URL environment variable {key} is not set"))
    } else {
        Ok(webhook.url.clone())
    }
}

fn has_blocker_for_scope(issues: &[ReconcileIssue], scope: &str) -> bool {
    issues
        .iter()
        .any(|issue| issue.severity == IssueSeverity::Blocker && issue.scope.starts_with(scope))
}

fn validate_pages_desired(desired: Option<&PagesConfig>) -> Vec<ReconcileIssue> {
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
    desired: &RepositoryIntegrationsCategory,
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
            || !autolink_actions.is_empty()
            || desired.policy.prune
        {
            issues.push(ReconcileIssue {
                scope: "integrations".to_owned(),
                severity: IssueSeverity::Blocker,
                message: "Webhook, deploy-key, Pages, autolink, and prune mutations require `policy.sensitive: true`.".to_owned(),
            });
        }
        webhook_actions.clear();
        deploy_key_actions.clear();
        *pages_action = None;
        autolink_actions.clear();
    }
}
