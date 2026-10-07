use serde::{Deserialize, Serialize};

use super::{ActorReference, CategoryPolicy};
use crate::config::manifest::RepositoryRuleConfig;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulesetsCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<RulesetReferenceV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repository_rulesets: Vec<RepositoryRulesetV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulesetReferenceV2 {
    pub name: String,
    pub target: String,
    pub enforcement: String,
    pub source_type: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRulesetV2 {
    pub name: String,
    pub target: String,
    pub enforcement: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions_json: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RepositoryRuleConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bypass_actors: Vec<RulesetBypassActorV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulesetBypassActorV2 {
    pub actor: ActorReference,

    #[serde(default = "default_bypass_mode")]
    pub bypass_mode: String,
}

fn default_bypass_mode() -> String {
    "always".to_owned()
}
