use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{CategoryPolicy, ReferencedResourceConfig};
use crate::config::manifest::RepositorySettingsConfig;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<RepositorySettingsConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<RepositoryMetadataConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<CustomPropertyValueConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub immutable_releases: Option<ImmutableReleasesConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferencedResourceConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryMetadataConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_template: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_forking: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomPropertyValueConfig {
    pub property_name: String,
    pub value: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableReleasesConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforced_by_owner: Option<bool>,
}
