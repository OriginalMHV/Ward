use super::normalize::{canonicalize_url, imported_webhook_identity, normalize_events};
use super::{
    CollectedAutolink, CollectedDeployKey, CollectedIntegrationsState, CollectedPages,
    CollectedWebhook, IntegrationsCollection,
};
use crate::config::manifest::{
    AutolinkConfig, DeployKeyConfig, ExternalValueReference, ManifestCategoryName, PagesConfig,
    RepositoryIntegrationsCategory, WebhookConfig,
};
use crate::github::Client;
use crate::github::actions::ReadOutcome;
use crate::reconcile::access_integrations::record_read_outcome;
use anyhow::Result;

const WEBHOOK_SECRET_HINT: &str =
    "GitHub does not return existing webhook secret values; preserve or rotate it explicitly.";

const DEPLOY_KEY_HINT: &str =
    "GitHub does not return deploy key material; provide replacement_key to rotate this key.";

pub async fn collect_integrations(
    client: &Client,
    repo: &str,
    desired: &RepositoryIntegrationsCategory,
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
                        config: WebhookConfig {
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
                    config: DeployKeyConfig {
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
                config: PagesConfig {
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
                    config: AutolinkConfig {
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

    let category = RepositoryIntegrationsCategory {
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
