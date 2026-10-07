use super::collect::collect_integrations;
use super::normalize::{
    canonicalize_url, normalized_autolink, normalized_pages, normalized_webhook,
    normalized_webhook_config,
};
use super::plan::{
    deploy_key_equivalent, match_deploy_key, read_env, resolve_required_external_value,
    resolve_required_url,
};
use super::{
    AutolinkAction, DeployKeyAction, IntegrationsApplyReport, IntegrationsCollection,
    IntegrationsPlan, IntegrationsVerification, PagesAction, WebhookAction,
};
use crate::config::manifest::{
    DeployKeyConfig, ExternalValueReference, ManagementDisposition, RepositoryIntegrationsCategory,
};
use crate::github::Client;
use crate::github::integrations::{WebhookConfigPatch, WebhookMetadataPatch};
use crate::reconcile::common::issue::{IssueSeverity, format_issue};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

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
    if !plan.policy.sensitive && !plan.is_empty() {
        report.blocked.push(
            "Webhook, deploy-key, Pages, and autolink mutations require `policy.sensitive: true`."
                .to_owned(),
        );
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
                    content_type: (current_normalized.content_type
                        != desired_normalized.content_type)
                        .then_some(desired_normalized.content_type.as_str()),
                    insecure_ssl: (current_normalized.insecure_ssl
                        != desired_normalized.insecure_ssl)
                        .then_some(desired_normalized.insecure_ssl),
                    secret: None,
                };
                let metadata_patch = WebhookMetadataPatch {
                    active: (current_normalized.active != desired_normalized.active)
                        .then_some(desired_normalized.active),
                    events: (current_normalized.events != desired_normalized.events)
                        .then_some(desired_normalized.events.as_slice()),
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
    desired: &RepositoryIntegrationsCategory,
) -> Result<IntegrationsVerification> {
    let current = collect_integrations(client, repo, desired).await?;
    Ok(verify_integrations_state(&current, desired))
}

pub fn verify_integrations_state(
    current: &IntegrationsCollection,
    desired: &RepositoryIntegrationsCategory,
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
            "Labels belong to the repository category, so the integrations category ignores them."
                .to_owned(),
        );
    }

    verification
}

async fn apply_deploy_key_create(
    client: &Client,
    repo: &str,
    desired: &DeployKeyConfig,
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

fn resolve_optional_external_value(
    reference: Option<&ExternalValueReference>,
) -> Result<Option<String>, String> {
    match reference {
        None => Ok(None),
        Some(ExternalValueReference::Env { key }) => read_env(key)
            .map(Some)
            .map_err(|_| format!("environment variable `{key}` is not set")),
        Some(ExternalValueReference::Manual { hint }) => Err(match hint {
            Some(hint) => format!("value must be provided manually ({hint})"),
            None => "value must be provided manually".to_owned(),
        }),
    }
}
