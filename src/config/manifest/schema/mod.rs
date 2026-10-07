use serde::{Deserialize, Serialize};

mod access;
mod actions;
mod branch_protection;
mod environments;
mod files;
mod integrations;
mod repository;
mod rulesets;
mod security;

pub use self::{
    access::*, actions::*, branch_protection::*, environments::*, files::*, integrations::*,
    repository::*, rulesets::*, security::*,
};

use crate::config::manifest::Manifest;

const MANIFEST_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ManifestDocument(pub Manifest);

impl ManifestDocument {
    pub fn render(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

impl std::ops::Deref for ManifestDocument {
    type Target = Manifest;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ManifestDocument {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<&Manifest> for ManifestDocument {
    fn from(manifest: &Manifest) -> Self {
        Self(manifest.clone())
    }
}

impl Manifest {
    pub fn to_document(&self) -> ManifestDocument {
        ManifestDocument::from(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSchema {
    pub version: u32,
}

impl ManifestSchema {
    pub const fn current() -> Self {
        Self {
            version: MANIFEST_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestProvenance {
    pub repository: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_node_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch_head_oid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestCategories {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<SecurityCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<RepositoryCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_protection: Option<BranchProtectionCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rulesets: Option<RulesetsCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<FilesCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionsCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environments: Option<EnvironmentsCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<RepositoryAccessCategory>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrations: Option<RepositoryIntegrationsCategory>,
}

impl ManifestCategories {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryPolicy {
    #[serde(default)]
    pub disposition: ManagementDisposition,

    #[serde(default)]
    pub prune: bool,

    #[serde(default)]
    pub sensitive: bool,
}

impl CategoryPolicy {
    pub fn managed() -> Self {
        Self {
            disposition: ManagementDisposition::Managed,
            prune: false,
            sensitive: false,
        }
    }

    pub fn observe() -> Self {
        Self::default()
    }

    pub fn observe_sensitive() -> Self {
        Self {
            sensitive: true,
            ..Self::default()
        }
    }
}

impl Default for CategoryPolicy {
    fn default() -> Self {
        Self {
            disposition: ManagementDisposition::Observe,
            prune: false,
            sensitive: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementDisposition {
    Managed,
    Reference,
    Placeholder,
    #[default]
    Observe,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEntry {
    pub category: ManifestCategoryName,
    pub endpoint: String,
    pub outcome: CoverageOutcome,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_permission: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestCategoryName {
    Security,
    Repository,
    BranchProtection,
    Rulesets,
    Files,
    Actions,
    Environments,
    Access,
    Integrations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageOutcome {
    Collected,
    Redacted,
    PermissionDenied,
    Unsupported,
    Unavailable,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamedValueConfig {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecretPlaceholderConfig {
    pub name: String,
    pub value_from: ExternalValueReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum ExternalValueReference {
    Env {
        key: String,
    },
    Manual {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReferencedResourceConfig {
    #[serde(rename = "type")]
    pub resource_type: ReferencedResourceType,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferencedResourceType {
    App,
    Team,
    Role,
    RunnerGroup,
    OrganizationSecret,
    OrganizationVariable,
    CodeSecurityConfiguration,
    ProtectionRule,
    /// A self-hosted runner observed on a repository. Ward only ever reads
    /// this as a diagnostic reference (name/labels/status); it never
    /// registers, re-registers, or deletes runners.
    Runner,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum ActorReference {
    OrganizationAdmin,
    Team {
        slug: String,
    },
    User {
        login: String,
    },
    App {
        slug: String,
    },
    Role {
        name: String,
    },
    Unresolved {
        actor_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<u64>,
    },
}
