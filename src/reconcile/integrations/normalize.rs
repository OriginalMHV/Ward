use super::CollectedWebhook;
use crate::config::manifest::{AutolinkConfig, ExternalValueReference, PagesConfig, WebhookConfig};

const WEBHOOK_URL_ENV_PREFIX: &str = "WARD_WEBHOOK_URL_";

pub(super) fn normalized_webhook(current: &CollectedWebhook) -> NormalizedWebhook {
    normalized_webhook_config(&current.config)
}

pub(super) fn normalized_webhook_config(webhook: &WebhookConfig) -> NormalizedWebhook {
    NormalizedWebhook {
        canonical_url: canonicalize_url(&webhook.url),
        active: webhook.active.unwrap_or(true),
        events: normalize_events(&webhook.events),
        content_type: webhook
            .content_type
            .clone()
            .unwrap_or_else(|| "form".to_owned()),
        insecure_ssl: webhook.insecure_ssl.unwrap_or(false),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedWebhook {
    pub(super) canonical_url: String,
    pub(super) active: bool,
    pub(super) events: Vec<String>,
    pub(super) content_type: String,
    pub(super) insecure_ssl: bool,
}

pub(super) fn normalized_pages(pages: &PagesConfig) -> NormalizedPages {
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
pub(super) struct NormalizedPages {
    pub(super) build_type: Option<String>,
    pub(super) source_branch: Option<String>,
    pub(super) source_path: Option<String>,
    pub(super) cname: Option<String>,
    pub(super) https_enforced: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedAutolink {
    pub(super) key_prefix: String,
    pub(super) url_template: String,
    pub(super) is_alphanumeric: bool,
}

pub(super) fn normalized_autolink(autolink: &AutolinkConfig) -> NormalizedAutolink {
    NormalizedAutolink {
        key_prefix: autolink.key_prefix.clone(),
        url_template: autolink.url_template.clone(),
        is_alphanumeric: autolink.is_alphanumeric.unwrap_or(true),
    }
}

pub(super) fn normalize_events(events: &[String]) -> Vec<String> {
    let mut events = if events.is_empty() {
        vec!["push".to_owned()]
    } else {
        events.to_vec()
    };
    events.sort();
    events.dedup();
    events
}

pub fn canonicalize_url(url: &str) -> String {
    if env_placeholder_key(url).is_some() {
        return url.to_owned();
    }
    match reqwest::Url::parse(url) {
        Ok(mut parsed) => {
            // Setters fail only for cannot-be-a-base URLs, which carry no credentials to remove.
            parsed.set_username("").ok();
            parsed.set_password(None).ok();
            if (parsed.scheme() == "http" && parsed.port() == Some(80))
                || (parsed.scheme() == "https" && parsed.port() == Some(443))
            {
                parsed.set_port(None).ok();
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

pub(super) fn env_placeholder_key(value: &str) -> Option<&str> {
    value
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
        .filter(|key| !key.is_empty())
}

pub(super) fn imported_webhook_identity(url: &str) -> (String, Option<ExternalValueReference>) {
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
            parsed.set_username("***").ok();
            parsed.set_password(None).ok();
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
