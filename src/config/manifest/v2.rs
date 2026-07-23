use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    BranchProtectionConfig, Manifest, RepositoryRuleConfig, RepositorySettingsConfig, TeamAccess,
};

const MANIFEST_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ManifestDocument(pub Manifest);

impl ManifestDocument {
    pub fn into_manifest(self) -> Manifest {
        self.0
    }

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
pub struct ManifestCategories {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<SecurityCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<RepositoryCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_protection: Option<BranchProtectionCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rulesets: Option<RulesetsCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<FilesCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionsCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environments: Option<EnvironmentsCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<RepositoryAccessCategoryV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrations: Option<RepositoryIntegrationsCategoryV2>,
}

impl ManifestCategories {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SecurityCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advanced_security: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_security: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependabot_alerts: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependabot_security_updates: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_push_protection: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_validity_checks: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_non_provider_patterns: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_ai_detection: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_alert_dismissal: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_bypass: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_alert_dismissal_options: Option<SecurityReviewerOptionsConfigV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_scanning_delegated_bypass_options: Option<SecurityReviewerOptionsConfigV2>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_vulnerability_reporting: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codeql_default_setup: Option<CodeqlDefaultSetupConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_reference: Option<ReferencedResourceConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegated_alert_dismissal_reviewers: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegated_bypass_reviewers: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferencedResourceConfig>,
}

impl SecurityCategoryV2 {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            advanced_security: None,
            code_security: None,
            dependabot_alerts: None,
            dependabot_security_updates: None,
            secret_scanning: None,
            secret_scanning_push_protection: None,
            secret_scanning_validity_checks: None,
            secret_scanning_non_provider_patterns: None,
            secret_scanning_ai_detection: None,
            secret_scanning_delegated_alert_dismissal: None,
            secret_scanning_delegated_bypass: None,
            secret_scanning_delegated_alert_dismissal_options: None,
            secret_scanning_delegated_bypass_options: None,
            private_vulnerability_reporting: None,
            codeql_default_setup: None,
            configuration_reference: None,
            delegated_alert_dismissal_reviewers: Vec::new(),
            delegated_bypass_reviewers: Vec::new(),
            references: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct SecurityReviewerOptionsConfigV2 {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<SecurityReviewerConfigV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SecurityReviewerConfigV2 {
    pub actor: ActorReference,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CodeqlDefaultSetupConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_suite: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_label: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
pub struct CustomPropertyValueConfig {
    pub property_name: String,
    pub value: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ImmutableReleasesConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforced_by_owner: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BranchProtectionCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<BranchProtectionConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch_detailed: Option<DetailedBranchProtectionConfigV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_branches: Vec<ProtectedBranchConfig>,
}

impl BranchProtectionCategoryV2 {
    pub fn observe() -> Self {
        Self {
            policy: CategoryPolicy::observe(),
            default_branch: None,
            default_branch_detailed: None,
            protected_branches: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct DetailedBranchProtectionConfigV2 {
    #[serde(default)]
    pub protection: BranchProtectionConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_check_contexts: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_checks: Vec<BranchStatusCheckConfigV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub push_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissal_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_request_bypass_allowances: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_last_push_approval: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_creations: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_reviewers: Option<Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_conversation_resolution: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_signed_commits: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_branch: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fork_syncing: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ProtectedBranchConfig {
    pub name: String,

    #[serde(default)]
    pub protection: BranchProtectionConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_check_contexts: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_checks: Vec<BranchStatusCheckConfigV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub push_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissal_restrictions: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_request_bypass_allowances: Vec<ActorReference>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_last_push_approval: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_creations: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_reviewers: Option<Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_conversation_resolution: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_signed_commits: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_branch: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fork_syncing: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BranchStatusCheckConfigV2 {
    pub context: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_slug: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RulesetsCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<RulesetReferenceV2>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repository_rulesets: Vec<RepositoryRulesetV2>,
}

impl RulesetsCategoryV2 {
    pub fn observe() -> Self {
        Self {
            policy: CategoryPolicy::observe(),
            references: Vec::new(),
            repository_rulesets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RulesetReferenceV2 {
    pub name: String,
    pub target: String,
    pub enforcement: String,
    pub source_type: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
pub struct RulesetBypassActorV2 {
    pub actor: ActorReference,

    #[serde(default = "default_bypass_mode")]
    pub bypass_mode: String,
}

fn default_bypass_mode() -> String {
    "always".to_owned()
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FilesCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ManagedFileV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ManagedFileV2 {
    pub path: String,
    pub content: String,

    #[serde(default)]
    pub encoding: FileEncoding,

    #[serde(default = "default_file_mode")]
    pub mode: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum FileEncoding {
    #[serde(rename = "utf-8")]
    #[default]
    Utf8,
    #[serde(rename = "base64")]
    Base64,
}

fn default_file_mode() -> String {
    "100644".to_owned()
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ActionsCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<ActionsSettingsConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<NamedValueConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretPlaceholderConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependabot_secrets: Vec<SecretPlaceholderConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codespaces_secrets: Vec<SecretPlaceholderConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<WorkflowStateConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferencedResourceConfig>,
}

impl ActionsCategoryV2 {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ActionsSettingsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_actions: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_actions: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_github_owned_actions: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_verified_creator_actions: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_pinned_actions: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workflow_permissions: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_approve_pull_request_reviews: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_retention_days: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_retention_days: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_fork_workflows_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_fork_workflow_approval: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_write_tokens_to_workflows: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_secrets_and_variables: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_approval_for_fork_pr_workflows: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_pull_request_workflows_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_pull_request_contributor_approval: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_access_level: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_subject_claim_template: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_use_default: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_use_immutable_subject: Option<bool>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oidc_subject_claim_include_keys: Vec<String>,

    /// `GET/PUT /repos/{owner}/{repo}/actions/cache/retention-limit`'s
    /// `max_cache_retention_days`. Distinct from `artifact_retention_days`/
    /// `log_retention_days` (a different, older endpoint) — this limits how
    /// long GitHub Actions *dependency caches* (`actions/cache`) may be
    /// retained before eviction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_retention_limit_days: Option<u32>,

    /// `GET/PUT /repos/{owner}/{repo}/actions/cache/storage-limit`'s
    /// `max_cache_size_gb`. This is a writable policy limit, not the
    /// current cache usage (`GET .../actions/cache/usage`, which is
    /// runtime data and is never part of desired configuration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_storage_limit_gb: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct EnvironmentsCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<EnvironmentConfigV2>,
}

impl EnvironmentsCategoryV2 {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct EnvironmentConfigV2 {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_timer_minutes: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prevent_self_review: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_policy: Option<EnvironmentDeploymentPolicyConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<EnvironmentReviewerConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protection_apps: Vec<ReferencedResourceConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<NamedValueConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretPlaceholderConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct EnvironmentDeploymentPolicyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_branches: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_branch_policies: Option<bool>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branch_patterns: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct EnvironmentReviewerConfig {
    pub actor: ActorReference,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct RepositoryAccessCategoryV2 {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<TeamAccess>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collaborators: Vec<CollaboratorAccessConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferencedResourceConfig>,
}

impl RepositoryAccessCategoryV2 {
    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CollaboratorAccessConfig {
    pub actor: ActorReference,
    pub permission: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
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
pub struct AutolinkConfigV2 {
    pub key_prefix: String,
    pub url_template: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_alphanumeric: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LabelConfigV2 {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NamedValueConfig {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SecretPlaceholderConfig {
    pub name: String,
    pub value_from: ExternalValueReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
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
pub struct WorkflowStateConfig {
    pub path: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
