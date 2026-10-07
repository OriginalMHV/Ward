use serde::{Deserialize, Serialize};

use super::{CategoryPolicy, ExternalValueReference};

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryIntegrationsCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhooks: Vec<WebhookConfigV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deploy_keys: Vec<DeployKeyConfigV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<PagesConfigV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub autolinks: Vec<AutolinkConfigV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<LabelConfigV2>,
}

impl RepositoryIntegrationsCategoryV2 {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookConfigV2 {
    pub url: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_from: Option<ExternalValueReference>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insecure_ssl: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<ExternalValueReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeployKeyConfigV2 {
    pub title: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_key: Option<ExternalValueReference>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PagesConfigV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_branch: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cname: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub https_enforced: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutolinkConfigV2 {
    pub key_prefix: String,
    pub url_template: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_alphanumeric: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LabelConfigV2 {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,
}
