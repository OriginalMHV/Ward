//! Security, rulesets, and legacy branch-protection reconciliation.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::manifest::{
    ActorReference, BranchProtectionCategoryV2, BranchProtectionConfig, BranchStatusCheckConfigV2,
    CategoryPolicy, CodeqlDefaultSetupConfig, CoverageEntry, CoverageOutcome,
    DetailedBranchProtectionConfigV2, ManagementDisposition, ManifestCategoryName,
    ProtectedBranchConfig, ReferencedResourceConfig, ReferencedResourceType, RepositoryRuleConfig,
    RepositoryRulesetV2, RulesetBypassActorV2, RulesetReferenceV2, RulesetsCategoryV2,
    SecurityCategoryV2, SecurityReviewerConfigV2, SecurityReviewerOptionsConfigV2,
};
use crate::github::Client;
use crate::github::branch_protection::{
    ActorSet, AppActor, DesiredBranchProtection, DetailedBranchProtection, StatusCheckRequirement,
    TeamActor, UserActor,
};
use crate::github::rulesets::{RulesetCustomRepositoryRole, RulesetDetail};
use crate::github::security::{
    CodeSecurityConfiguration, CodeqlDefaultSetupState, RepositoryCodeSecurityConfiguration,
    RepositorySecurityBaseline, SecurityAndAnalysisState,
};

const SECURITY_ATTACHED_CONFIGURATION_PATH: &str = "categories.security.configuration_reference";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileIssueSeverity {
    Warning,
    Blocker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileIssue {
    pub resource: Option<String>,
    pub code: &'static str,
    pub severity: ReconcileIssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecurityCollection {
    pub repository_id: u64,
    pub category: SecurityCategoryV2,
    pub analysis: SecurityAndAnalysisState,
    pub private_vulnerability_reporting: Option<bool>,
    pub codeql_default_setup: Option<CodeqlDefaultSetupState>,
    pub attached_configuration: Option<RepositoryCodeSecurityConfiguration>,
    pub available_configurations: Vec<CodeSecurityConfiguration>,
    pub team_ids_by_slug: HashMap<String, u64>,
    pub repository_role_ids_by_name: HashMap<String, u64>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityPlan {
    pub repository_id: u64,
    pub patch_security_and_analysis: Option<serde_json::Value>,
    pub dependabot_alerts: Option<bool>,
    pub dependabot_security_updates: Option<bool>,
    pub private_vulnerability_reporting: Option<bool>,
    pub codeql_default_setup: Option<CodeqlDefaultSetupState>,
    pub attach_configuration_id: Option<u64>,
    pub detach_configuration: bool,
    pub issues: Vec<ReconcileIssue>,
}

impl SecurityPlan {
    pub fn has_changes(&self) -> bool {
        self.patch_security_and_analysis.is_some()
            || self.dependabot_alerts.is_some()
            || self.dependabot_security_updates.is_some()
            || self.private_vulnerability_reporting.is_some()
            || self.codeql_default_setup.is_some()
            || self.attach_configuration_id.is_some()
            || self.detach_configuration
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityApplyResult {
    pub applied_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityVerifyResult {
    pub matches: bool,
    pub plan: SecurityPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetReference {
    pub name: String,
    pub target: String,
    pub enforcement: String,
    pub source_type: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActualRepositoryRuleset {
    pub id: u64,
    pub source_type: String,
    pub source: String,
    pub ruleset: RepositoryRulesetV2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RulesetsCollection {
    pub category: RulesetsCategoryV2,
    pub actual_repository_rulesets: Vec<ActualRepositoryRuleset>,
    pub inherited_rulesets: Vec<RulesetReference>,
    pub team_ids_by_slug: HashMap<String, u64>,
    pub repository_role_ids_by_name: HashMap<String, u64>,
    pub app_ids_by_slug: HashMap<String, u64>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RulesetPlanAction {
    Create {
        ruleset: RepositoryRulesetV2,
    },
    Update {
        ruleset_id: u64,
        ruleset: RepositoryRulesetV2,
    },
    Delete {
        ruleset_id: u64,
        name: String,
    },
    Unchanged {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RulesetsPlan {
    pub actions: Vec<RulesetPlanAction>,
    pub issues: Vec<ReconcileIssue>,
}

impl RulesetsPlan {
    pub fn has_changes(&self) -> bool {
        self.actions
            .iter()
            .any(|action| !matches!(action, RulesetPlanAction::Unchanged { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetsApplyResult {
    pub applied_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RulesetsVerifyResult {
    pub matches: bool,
    pub plan: RulesetsPlan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActualProtectedBranch {
    pub name: String,
    pub is_default_branch: bool,
    pub manifest: ProtectedBranchConfig,
    pub raw: DetailedBranchProtection,
    pub status_checks: Vec<StatusCheckRequirement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BranchProtectionCollection {
    pub default_branch_name: String,
    pub category: BranchProtectionCategoryV2,
    pub actual_branches: Vec<ActualProtectedBranch>,
    pub app_ids_by_slug: HashMap<String, i64>,
    pub app_slugs_by_id: HashMap<i64, String>,
    pub coverage: Vec<CoverageEntry>,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchProtectionPlanAction {
    Upsert {
        branch: String,
        desired: Box<DesiredBranchProtection>,
    },
    Delete {
        branch: String,
    },
    Unchanged {
        branch: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchProtectionPlan {
    pub actions: Vec<BranchProtectionPlanAction>,
    pub issues: Vec<ReconcileIssue>,
}

impl BranchProtectionPlan {
    pub fn has_changes(&self) -> bool {
        self.actions
            .iter()
            .any(|action| !matches!(action, BranchProtectionPlanAction::Unchanged { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchProtectionApplyResult {
    pub applied_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchProtectionVerifyResult {
    pub matches: bool,
    pub plan: BranchProtectionPlan,
}

pub async fn collect_security_category(
    client: &Client,
    repo: &str,
    category: Option<&SecurityCategoryV2>,
) -> Result<SecurityCollection> {
    let RepositorySecurityBaseline {
        id: repository_id,
        security_and_analysis,
    } = client.get_repository_security_baseline(repo).await?;
    let mut issues = Vec::new();
    let mut coverage = vec![collected_entry(
        ManifestCategoryName::Security,
        "GET /repos/{owner}/{repo}",
    )];
    let analysis = security_and_analysis.unwrap_or_default();

    let dependabot_alerts = match client.read_dependabot_alerts_state(repo).await? {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/vulnerability-alerts",
                reason,
            ));
            None
        }
    };

    let dependabot_security_updates_endpoint =
        match client.read_dependabot_security_updates_state(repo).await? {
            crate::github::actions::ReadOutcome::Available(value) => {
                coverage.push(collected_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                ));
                Some(value)
            }
            crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
                coverage.push(permission_denied_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                    reason,
                ));
                None
            }
            crate::github::actions::ReadOutcome::NotApplicable(reason) => {
                coverage.push(not_applicable_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                    reason,
                ));
                None
            }
            crate::github::actions::ReadOutcome::Unavailable(reason) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Security,
                    "GET /repos/{owner}/{repo}/automated-security-fixes",
                    reason,
                ));
                None
            }
        };

    let private_vulnerability_reporting = match client
        .read_private_vulnerability_reporting_status(repo)
        .await?
    {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/private-vulnerability-reporting",
                reason,
            ));
            None
        }
    };

    let codeql_default_setup = match client.read_codeql_default_setup(repo).await? {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-scanning/default-setup",
                reason,
            ));
            None
        }
    };

    let attached_configuration = match client
        .read_repository_code_security_configuration(repo)
        .await?
    {
        crate::github::actions::ReadOutcome::Available(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
            ));
            Some(value)
        }
        crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(permission_denied_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(not_applicable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
                reason,
            ));
            None
        }
        crate::github::actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /repos/{owner}/{repo}/code-security-configuration",
                reason,
            ));
            None
        }
    };

    let available_configurations = match client.list_code_security_configurations().await {
        Ok(mut value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Security,
                "GET /orgs/{org}/code-security/configurations",
            ));
            value.sort_by(|left, right| left.name.cmp(&right.name));
            value
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Security,
                "GET /orgs/{org}/code-security/configurations",
                error.to_string(),
            ));
            Vec::new()
        }
    };

    let configuration_reference = attached_configuration
        .as_ref()
        .map(configuration_reference_from_attachment);

    let mut team_ids_by_slug = HashMap::new();
    let mut repository_role_ids_by_name = HashMap::new();
    if analysis
        .secret_scanning_delegated_alert_dismissal_options
        .is_some()
        || analysis.secret_scanning_delegated_bypass_options.is_some()
    {
        match client.list_org_teams().await {
            Ok(teams) => {
                for team in teams {
                    team_ids_by_slug.insert(team.slug, team.id);
                }
            }
            Err(error) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Security,
                    "GET /orgs/{org}/teams",
                    error.to_string(),
                ));
                issues.push(warning_issue(
                    Some(repo.to_owned()),
                    "security-reviewer-team-resolution",
                    format!("Could not resolve delegated reviewer team IDs for {repo}: {error}"),
                ));
            }
        }
        match client.list_ruleset_custom_repository_roles().await {
            Ok(roles) => {
                for (id, name) in repository_role_lookup(&roles) {
                    repository_role_ids_by_name.insert(name, id);
                }
            }
            Err(error) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Security,
                    "GET /orgs/{org}/custom-repository-roles",
                    error.to_string(),
                ));
                issues.push(warning_issue(
                    Some(repo.to_owned()),
                    "security-reviewer-role-resolution",
                    format!("Could not resolve delegated reviewer role IDs for {repo}: {error}"),
                ));
            }
        }
    }

    let delegated_alert_dismissal_options = analysis
        .secret_scanning_delegated_alert_dismissal_options
        .as_ref()
        .map(|options| {
            reviewer_options_from_api(
                options,
                &team_ids_by_slug,
                &repository_role_ids_by_name,
                repo,
                &mut issues,
            )
        })
        .transpose()?;
    let delegated_bypass_options = analysis
        .secret_scanning_delegated_bypass_options
        .as_ref()
        .map(|options| {
            reviewer_options_from_api(
                options,
                &team_ids_by_slug,
                &repository_role_ids_by_name,
                repo,
                &mut issues,
            )
        })
        .transpose()?;

    let delegated_alert_dismissal_reviewers = delegated_alert_dismissal_options
        .as_ref()
        .map(|options| {
            options
                .reviewers
                .iter()
                .map(|reviewer| reviewer.actor.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let delegated_bypass_reviewers = delegated_bypass_options
        .as_ref()
        .map(|options| {
            options
                .reviewers
                .iter()
                .map(|reviewer| reviewer.actor.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let references = configuration_reference.iter().cloned().collect::<Vec<_>>();

    let policy = category
        .map(|value| value.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    Ok(SecurityCollection {
        repository_id,
        category: SecurityCategoryV2 {
            policy,
            advanced_security: bool_field(&analysis.advanced_security),
            code_security: bool_field(&analysis.code_security),
            dependabot_alerts,
            dependabot_security_updates: dependabot_security_updates_endpoint
                .or_else(|| bool_field(&analysis.dependabot_security_updates)),
            secret_scanning: bool_field(&analysis.secret_scanning),
            secret_scanning_push_protection: bool_field(&analysis.secret_scanning_push_protection),
            secret_scanning_validity_checks: bool_field(&analysis.secret_scanning_validity_checks),
            secret_scanning_non_provider_patterns: bool_field(
                &analysis.secret_scanning_non_provider_patterns,
            ),
            secret_scanning_ai_detection: bool_field(&analysis.secret_scanning_ai_detection),
            secret_scanning_delegated_alert_dismissal: bool_field(
                &analysis.secret_scanning_delegated_alert_dismissal,
            ),
            secret_scanning_delegated_bypass: bool_field(
                &analysis.secret_scanning_delegated_bypass,
            ),
            secret_scanning_delegated_alert_dismissal_options: delegated_alert_dismissal_options,
            secret_scanning_delegated_bypass_options: delegated_bypass_options,
            private_vulnerability_reporting,
            codeql_default_setup: codeql_default_setup.as_ref().map(codeql_state_to_manifest),
            configuration_reference,
            delegated_alert_dismissal_reviewers,
            delegated_bypass_reviewers,
            references,
        },
        analysis,
        private_vulnerability_reporting,
        codeql_default_setup,
        attached_configuration,
        available_configurations,
        team_ids_by_slug,
        repository_role_ids_by_name,
        coverage,
        issues,
    })
}

pub fn plan_security_category(
    desired: &SecurityCategoryV2,
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
                    SecurityReviewerOptionsConfigV2 {
                        reviewers: desired
                            .delegated_alert_dismissal_reviewers
                            .iter()
                            .cloned()
                            .map(|actor| SecurityReviewerConfigV2 { actor, mode: None })
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
                    SecurityReviewerOptionsConfigV2 {
                        reviewers: desired
                            .delegated_bypass_reviewers
                            .iter()
                            .cloned()
                            .map(|actor| SecurityReviewerConfigV2 { actor, mode: None })
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

        if let Some(value) = desired.private_vulnerability_reporting {
            if actual.private_vulnerability_reporting != Some(value) {
                private_vulnerability_reporting = Some(value);
            }
        }

        if let Some(codeql) = &desired.codeql_default_setup {
            let desired_codeql = codeql_manifest_to_state(codeql);
            if actual
                .codeql_default_setup
                .as_ref()
                .map(codeql_state_to_manifest)
                != Some(codeql.clone())
            {
                codeql_default_setup = Some(desired_codeql.clone());
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

pub async fn apply_security_plan(
    client: &Client,
    repo: &str,
    plan: &SecurityPlan,
) -> Result<SecurityApplyResult> {
    if let Some(issue) = plan
        .issues
        .iter()
        .find(|issue| issue.severity == ReconcileIssueSeverity::Blocker)
    {
        anyhow::bail!("Security plan is blocked: {}", issue.message);
    }

    let mut applied_steps = Vec::new();

    if plan.detach_configuration {
        client
            .detach_code_security_configurations(&[plan.repository_id])
            .await?;
        applied_steps.push("detach_code_security_configuration".to_owned());
    }
    if let Some(configuration_id) = plan.attach_configuration_id {
        client
            .attach_code_security_configuration(configuration_id, plan.repository_id)
            .await?;
        applied_steps.push(format!(
            "attach_code_security_configuration:{configuration_id}"
        ));
    }
    if let Some(enabled) = plan.dependabot_alerts {
        if enabled {
            client.enable_dependabot_alerts(repo).await?;
        } else {
            client.disable_dependabot_alerts(repo).await?;
        }
        applied_steps.push("dependabot_alerts".to_owned());
    }
    if let Some(enabled) = plan.dependabot_security_updates {
        if enabled {
            client.enable_dependabot_security_updates(repo).await?;
        } else {
            client.disable_dependabot_security_updates(repo).await?;
        }
        applied_steps.push("dependabot_security_updates".to_owned());
    }
    if let Some(value) = &plan.patch_security_and_analysis {
        client
            .update_repository_security_and_analysis(repo, value)
            .await?;
        applied_steps.push("security_and_analysis".to_owned());
    }
    if let Some(enabled) = plan.private_vulnerability_reporting {
        client
            .set_private_vulnerability_reporting(repo, enabled)
            .await?;
        applied_steps.push("private_vulnerability_reporting".to_owned());
    }
    if let Some(codeql) = &plan.codeql_default_setup {
        client.update_codeql_default_setup(repo, codeql).await?;
        applied_steps.push("codeql_default_setup".to_owned());
    }

    Ok(SecurityApplyResult { applied_steps })
}

pub async fn verify_security_category(
    client: &Client,
    repo: &str,
    desired: &SecurityCategoryV2,
) -> Result<SecurityVerifyResult> {
    for attempt in 0..10 {
        let actual = collect_security_category(client, repo, Some(desired)).await?;
        let plan = plan_security_category(desired, &actual)?;
        let waiting_on_codeql = plan.codeql_default_setup.is_some()
            && actual
                .codeql_default_setup
                .as_ref()
                .and_then(|value| value.state.as_deref())
                .is_some_and(|state| {
                    matches!(state, "queued" | "pending" | "in_progress" | "configuring")
                });
        let blocked = plan
            .issues
            .iter()
            .any(|issue| issue.severity == ReconcileIssueSeverity::Blocker);
        if !plan.has_changes() && !blocked {
            return Ok(SecurityVerifyResult {
                matches: true,
                plan,
            });
        }
        if waiting_on_codeql && attempt < 9 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        return Ok(SecurityVerifyResult {
            matches: false,
            plan,
        });
    }

    unreachable!()
}

pub async fn collect_rulesets_category(
    client: &Client,
    repo: &str,
    category: Option<&RulesetsCategoryV2>,
) -> Result<RulesetsCollection> {
    let mut issues = Vec::new();
    let mut coverage = vec![collected_entry(
        ManifestCategoryName::Rulesets,
        "GET /repos/{owner}/{repo}/rulesets",
    )];

    let all_rulesets = client.list_rulesets(repo).await?;
    let org_teams = match client.list_org_teams().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/teams",
            ));
            value
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/teams",
                error.to_string(),
            ));
            Vec::new()
        }
    };
    let custom_roles = match client.list_ruleset_custom_repository_roles().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/custom-repository-roles",
            ));
            value
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/custom-repository-roles",
                error.to_string(),
            ));
            Vec::new()
        }
    };
    let installed_apps = match client.list_org_installations().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/installations",
            ));
            value
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Rulesets,
                "GET /orgs/{org}/installations",
                error.to_string(),
            ));
            Vec::new()
        }
    };

    let team_by_id: HashMap<u64, String> = org_teams
        .into_iter()
        .map(|team| (team.id, team.slug))
        .collect();
    let app_by_id: HashMap<u64, String> = installed_apps
        .into_iter()
        .map(|app| (app.app_id, app.app_slug))
        .collect();
    let role_by_id = repository_role_lookup(&custom_roles);
    let user_by_id: HashMap<u64, String> = match client.list_ruleset_repo_collaborators(repo).await
    {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::Rulesets,
                "GET /repos/{owner}/{repo}/collaborators?affiliation=all",
            ));
            value
                .into_iter()
                .map(|user| (user.id, user.login))
                .collect()
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::Rulesets,
                "GET /repos/{owner}/{repo}/collaborators?affiliation=all",
                error.to_string(),
            ));
            HashMap::new()
        }
    };

    let mut actual_repository_rulesets = Vec::new();
    let mut inherited_rulesets = Vec::new();

    for ruleset in all_rulesets {
        if !ruleset.source_type.is_empty() && ruleset.source_type != "Repository" {
            inherited_rulesets.push(RulesetReference {
                name: ruleset.name,
                target: ruleset.target,
                enforcement: ruleset.enforcement,
                source_type: ruleset.source_type,
                source: ruleset.source,
            });
            continue;
        }

        let detail = client
            .get_ruleset(repo, ruleset.id)
            .await
            .map_err(|error| {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::Rulesets,
                    "GET /repos/{owner}/{repo}/rulesets/{ruleset_id}",
                    error.to_string(),
                ));
                issues.push(blocker_issue(
                    Some(ruleset.name.clone()),
                    "rulesets-detail-unavailable",
                    format!(
                        "Could not read full ruleset detail for {}: {error}. Planning updates from summary data would be unsafe.",
                        ruleset.name
                    ),
                ));
                error
            })
            .unwrap_or(RulesetDetail {
                id: ruleset.id,
                name: ruleset.name.clone(),
                enforcement: ruleset.enforcement.clone(),
                target: ruleset.target.clone(),
                rules: ruleset.rules.clone(),
                conditions: ruleset.conditions.clone(),
                bypass_actors: ruleset.bypass_actors.clone(),
            });
        let resolved = collect_repository_ruleset(
            detail,
            &team_by_id,
            &app_by_id,
            &role_by_id,
            &user_by_id,
            &ruleset.name,
            &mut issues,
        )?;
        actual_repository_rulesets.push(ActualRepositoryRuleset {
            id: ruleset.id,
            source_type: ruleset.source_type,
            source: ruleset.source,
            ruleset: resolved,
        });
    }

    actual_repository_rulesets.sort_by(|left, right| left.ruleset.name.cmp(&right.ruleset.name));
    inherited_rulesets.sort_by(|left, right| left.name.cmp(&right.name));

    let policy = category
        .map(|value| value.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    Ok(RulesetsCollection {
        category: RulesetsCategoryV2 {
            policy,
            references: inherited_rulesets
                .iter()
                .map(|reference| RulesetReferenceV2 {
                    name: reference.name.clone(),
                    target: reference.target.clone(),
                    enforcement: reference.enforcement.clone(),
                    source_type: reference.source_type.clone(),
                    source: reference.source.clone(),
                })
                .collect(),
            repository_rulesets: actual_repository_rulesets
                .iter()
                .map(|ruleset| ruleset.ruleset.clone())
                .collect(),
        },
        actual_repository_rulesets,
        inherited_rulesets,
        team_ids_by_slug: team_by_id
            .iter()
            .map(|(id, slug)| (slug.clone(), *id))
            .collect(),
        repository_role_ids_by_name: role_by_id
            .iter()
            .map(|(id, name)| (name.clone(), *id))
            .collect(),
        app_ids_by_slug: app_by_id
            .iter()
            .map(|(id, slug)| (slug.clone(), *id))
            .collect(),
        coverage,
        issues,
    })
}

pub fn plan_rulesets_category(
    desired: &RulesetsCategoryV2,
    actual: &RulesetsCollection,
) -> Result<RulesetsPlan> {
    let mut issues = actual.issues.clone();
    let mut actions = Vec::new();

    let actual_by_name: BTreeMap<&str, &ActualRepositoryRuleset> = actual
        .actual_repository_rulesets
        .iter()
        .map(|ruleset| (ruleset.ruleset.name.as_str(), ruleset))
        .collect();
    let desired_names: BTreeSet<&str> = desired
        .repository_rulesets
        .iter()
        .map(|ruleset| ruleset.name.as_str())
        .collect();

    if desired.policy.disposition != ManagementDisposition::Managed {
        for ruleset in &actual.actual_repository_rulesets {
            actions.push(RulesetPlanAction::Unchanged {
                name: ruleset.ruleset.name.clone(),
            });
        }
        return Ok(RulesetsPlan { actions, issues });
    }

    if normalize_ruleset_references(&desired.references)
        != normalize_ruleset_references(&actual.category.references)
    {
        issues.push(blocker_issue(
            Some("categories.rulesets.references".to_owned()),
            "rulesets-inherited-reference-drift",
            "Inherited ruleset references differ from the live repository state and are reference-only at repository scope.".to_owned(),
        ));
    }

    for desired_ruleset in &desired.repository_rulesets {
        validate_ruleset_bypass_actors(desired_ruleset, &mut issues)?;
        match actual_by_name.get(desired_ruleset.name.as_str()) {
            None => actions.push(RulesetPlanAction::Create {
                ruleset: desired_ruleset.clone(),
            }),
            Some(existing) => {
                if repository_ruleset_matches(desired_ruleset, &existing.ruleset) {
                    actions.push(RulesetPlanAction::Unchanged {
                        name: desired_ruleset.name.clone(),
                    });
                } else {
                    actions.push(RulesetPlanAction::Update {
                        ruleset_id: existing.id,
                        ruleset: desired_ruleset.clone(),
                    });
                }
            }
        }
    }

    if desired.policy.prune {
        for existing in &actual.actual_repository_rulesets {
            if !desired_names.contains(existing.ruleset.name.as_str()) {
                actions.push(RulesetPlanAction::Delete {
                    ruleset_id: existing.id,
                    name: existing.ruleset.name.clone(),
                });
            }
        }
    }

    actions.sort_by_key(ruleset_action_sort_key);

    if !actual.inherited_rulesets.is_empty() {
        issues.push(warning_issue(
            Some("categories.rulesets.references".to_owned()),
            "rulesets-inherited-reference-only",
            "Inherited rulesets are recorded as references only and will never be mutated by repository reconciliation.".to_owned(),
        ));
    }

    if actions
        .iter()
        .any(|action| !matches!(action, RulesetPlanAction::Unchanged { .. }))
        && !desired.policy.sensitive
    {
        issues.push(blocker_issue(
            Some("categories.rulesets.policy.sensitive".to_owned()),
            "rulesets-sensitive-gate",
            "Managing repository rulesets requires policy.sensitive = true".to_owned(),
        ));
    }

    Ok(RulesetsPlan { actions, issues })
}

pub async fn apply_rulesets_plan(
    client: &Client,
    repo: &str,
    plan: &RulesetsPlan,
) -> Result<RulesetsApplyResult> {
    if let Some(issue) = plan
        .issues
        .iter()
        .find(|issue| issue.severity == ReconcileIssueSeverity::Blocker)
    {
        anyhow::bail!("Rulesets plan is blocked: {}", issue.message);
    }

    let mut applied_steps = Vec::new();
    let requires_role_lookup = plan.actions.iter().any(|action| {
        matches!(
            action,
            RulesetPlanAction::Create { ruleset } | RulesetPlanAction::Update { ruleset, .. }
                if ruleset
                    .bypass_actors
                    .iter()
                    .any(|actor| matches!(&actor.actor, ActorReference::Role { .. }))
        )
    });
    let requires_app_lookup = plan.actions.iter().any(|action| {
        matches!(
            action,
            RulesetPlanAction::Create { ruleset } | RulesetPlanAction::Update { ruleset, .. }
                if ruleset
                    .bypass_actors
                    .iter()
                    .any(|actor| matches!(&actor.actor, ActorReference::App { .. }))
        )
    });
    let role_lookup = if requires_role_lookup {
        repository_role_lookup(
            &client
                .list_ruleset_custom_repository_roles()
                .await
                .context(
                    "Failed to resolve repository roles required by ruleset create/update actions",
                )?,
        )
    } else {
        repository_role_lookup(&[])
    };
    let app_lookup: HashMap<String, u64> = if requires_app_lookup {
        client
            .list_org_installations()
            .await
            .context("Failed to resolve installed apps required by ruleset create/update actions")?
            .into_iter()
            .map(|app| (app.app_slug, app.app_id))
            .collect()
    } else {
        HashMap::new()
    };

    for action in &plan.actions {
        match action {
            RulesetPlanAction::Create { ruleset } => {
                let body =
                    repository_ruleset_to_api_json(client, ruleset, &role_lookup, &app_lookup)
                        .await?;
                client.create_ruleset(repo, &body).await?;
                applied_steps.push(format!("create:{}", ruleset.name));
            }
            RulesetPlanAction::Update {
                ruleset_id,
                ruleset,
            } => {
                let body =
                    repository_ruleset_to_api_json(client, ruleset, &role_lookup, &app_lookup)
                        .await?;
                client.update_ruleset(repo, *ruleset_id, &body).await?;
                applied_steps.push(format!("update:{}", ruleset.name));
            }
            RulesetPlanAction::Delete { ruleset_id, name } => {
                client.delete_ruleset(repo, *ruleset_id).await?;
                applied_steps.push(format!("delete:{name}"));
            }
            RulesetPlanAction::Unchanged { .. } => {}
        }
    }

    Ok(RulesetsApplyResult { applied_steps })
}

pub async fn verify_rulesets_category(
    client: &Client,
    repo: &str,
    desired: &RulesetsCategoryV2,
) -> Result<RulesetsVerifyResult> {
    let actual = collect_rulesets_category(client, repo, Some(desired)).await?;
    let plan = plan_rulesets_category(desired, &actual)?;
    let blocked = plan
        .issues
        .iter()
        .any(|issue| issue.severity == ReconcileIssueSeverity::Blocker);
    Ok(RulesetsVerifyResult {
        matches: !plan.has_changes() && !blocked,
        plan,
    })
}

pub async fn collect_branch_protection_category(
    client: &Client,
    repo: &str,
    category: Option<&BranchProtectionCategoryV2>,
) -> Result<BranchProtectionCollection> {
    let repository = client.get_repo(repo).await?;
    let default_branch_name = repository.default_branch;
    let branches = client.list_protected_branches(repo).await?;

    let issues = Vec::new();
    let mut coverage = vec![
        collected_entry(
            ManifestCategoryName::BranchProtection,
            "GET /repos/{owner}/{repo}",
        ),
        collected_entry(
            ManifestCategoryName::BranchProtection,
            "GET /repos/{owner}/{repo}/branches?protected=true",
        ),
    ];
    let mut actual_branches = Vec::new();
    let app_slugs_by_id: HashMap<i64, String> = match client.list_org_installations().await {
        Ok(value) => {
            coverage.push(collected_entry(
                ManifestCategoryName::BranchProtection,
                "GET /orgs/{org}/installations",
            ));
            value
                .into_iter()
                .map(|app| (app.app_id as i64, app.app_slug))
                .collect()
        }
        Err(error) => {
            coverage.push(unavailable_entry(
                ManifestCategoryName::BranchProtection,
                "GET /orgs/{org}/installations",
                error.to_string(),
            ));
            HashMap::new()
        }
    };

    for branch in branches {
        let detail = match client
            .read_branch_protection_detail(repo, &branch.name)
            .await?
        {
            crate::github::actions::ReadOutcome::Available(detail) => {
                coverage.push(collected_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                ));
                detail
            }
            crate::github::actions::ReadOutcome::NotApplicable(reason) => {
                coverage.push(not_applicable_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                    reason,
                ));
                continue;
            }
            crate::github::actions::ReadOutcome::PermissionDenied(reason) => {
                coverage.push(permission_denied_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                    reason,
                ));
                continue;
            }
            crate::github::actions::ReadOutcome::Unavailable(reason) => {
                coverage.push(unavailable_entry(
                    ManifestCategoryName::BranchProtection,
                    "GET /repos/{owner}/{repo}/branches/{branch}/protection",
                    reason,
                ));
                continue;
            }
        };

        let manifest = protected_branch_from_detail(&branch.name, &detail, &app_slugs_by_id);
        let status_checks = detail
            .required_status_checks
            .as_ref()
            .map_or_else(Vec::new, |value| {
                if value.checks.is_empty() {
                    value
                        .contexts
                        .iter()
                        .map(|context| StatusCheckRequirement {
                            context: context.clone(),
                            app_id: None,
                        })
                        .collect::<Vec<_>>()
                } else {
                    value.checks.clone()
                }
            });

        actual_branches.push(ActualProtectedBranch {
            name: branch.name.clone(),
            is_default_branch: branch.name == default_branch_name,
            manifest,
            raw: detail,
            status_checks,
        });
    }

    actual_branches.sort_by(|left, right| left.name.cmp(&right.name));

    let default_branch = actual_branches
        .iter()
        .find(|branch| branch.is_default_branch)
        .map(|branch| branch.manifest.protection.clone());
    let default_branch_detailed = actual_branches
        .iter()
        .find(|branch| branch.is_default_branch)
        .map(|branch| detailed_branch_config_from_manifest(&branch.manifest));
    let protected_branches = actual_branches
        .iter()
        .filter(|branch| !branch.is_default_branch)
        .map(|branch| branch.manifest.clone())
        .collect();

    let policy = category
        .map(|value| value.policy.clone())
        .unwrap_or_else(CategoryPolicy::observe_sensitive);

    Ok(BranchProtectionCollection {
        default_branch_name,
        category: BranchProtectionCategoryV2 {
            policy,
            default_branch,
            default_branch_detailed,
            protected_branches,
        },
        actual_branches,
        app_ids_by_slug: app_slugs_by_id
            .iter()
            .map(|(id, slug)| (slug.clone(), *id))
            .collect(),
        app_slugs_by_id,
        coverage,
        issues,
    })
}

pub fn plan_branch_protection_category(
    desired: &BranchProtectionCategoryV2,
    actual: &BranchProtectionCollection,
) -> Result<BranchProtectionPlan> {
    let mut issues = actual.issues.clone();
    let mut actions = Vec::new();

    let actual_by_name: BTreeMap<&str, &ActualProtectedBranch> = actual
        .actual_branches
        .iter()
        .map(|branch| (branch.name.as_str(), branch))
        .collect();
    let mut desired_by_name: BTreeMap<String, DesiredBranchProtection> = BTreeMap::new();

    if desired.policy.disposition != ManagementDisposition::Managed {
        for branch in &actual.actual_branches {
            actions.push(BranchProtectionPlanAction::Unchanged {
                branch: branch.name.clone(),
            });
        }
        return Ok(BranchProtectionPlan { actions, issues });
    }

    if let Some(config) = &desired.default_branch_detailed {
        let existing = actual_by_name
            .get(actual.default_branch_name.as_str())
            .copied();
        let plan = desired_branch_protection_from_detailed_manifest(
            &actual.default_branch_name,
            config,
            existing,
            &actual.app_ids_by_slug,
            &mut issues,
        )?;
        desired_by_name.insert(actual.default_branch_name.clone(), plan);
    } else if let Some(config) = &desired.default_branch {
        let existing = actual_by_name
            .get(actual.default_branch_name.as_str())
            .copied();
        let plan = desired_branch_protection_from_default(
            &actual.default_branch_name,
            config,
            existing,
            &actual.app_ids_by_slug,
            &mut issues,
        )?;
        desired_by_name.insert(actual.default_branch_name.clone(), plan);
    }
    for protected_branch in &desired.protected_branches {
        let existing = actual_by_name.get(protected_branch.name.as_str()).copied();
        let plan = desired_branch_protection_from_manifest(
            protected_branch,
            existing,
            &actual.app_ids_by_slug,
            &mut issues,
        )?;
        desired_by_name.insert(protected_branch.name.clone(), plan);
    }

    for (branch, desired_branch) in &desired_by_name {
        if let Some(existing) = actual_by_name.get(branch.as_str()) {
            if protected_branch_matches(&existing.manifest, desired_branch) {
                actions.push(BranchProtectionPlanAction::Unchanged {
                    branch: branch.clone(),
                });
            } else {
                actions.push(BranchProtectionPlanAction::Upsert {
                    branch: branch.clone(),
                    desired: Box::new(desired_branch.clone()),
                });
            }
        } else {
            actions.push(BranchProtectionPlanAction::Upsert {
                branch: branch.clone(),
                desired: Box::new(desired_branch.clone()),
            });
        }
    }

    if desired.policy.prune {
        for branch in &actual.actual_branches {
            if !desired_by_name.contains_key(&branch.name) {
                actions.push(BranchProtectionPlanAction::Delete {
                    branch: branch.name.clone(),
                });
            }
        }
    }

    actions.sort_by_key(branch_action_sort_key);

    if actions
        .iter()
        .any(|action| !matches!(action, BranchProtectionPlanAction::Unchanged { .. }))
        && !desired.policy.sensitive
    {
        issues.push(blocker_issue(
            Some("categories.branch_protection.policy.sensitive".to_owned()),
            "branch-protection-sensitive-gate",
            "Managing legacy branch protection requires policy.sensitive = true".to_owned(),
        ));
    }

    Ok(BranchProtectionPlan { actions, issues })
}

pub async fn apply_branch_protection_plan(
    client: &Client,
    repo: &str,
    plan: &BranchProtectionPlan,
) -> Result<BranchProtectionApplyResult> {
    if let Some(issue) = plan
        .issues
        .iter()
        .find(|issue| issue.severity == ReconcileIssueSeverity::Blocker)
    {
        anyhow::bail!("Branch protection plan is blocked: {}", issue.message);
    }

    let mut applied_steps = Vec::new();
    for action in &plan.actions {
        match action {
            BranchProtectionPlanAction::Upsert { branch, desired } => {
                client
                    .update_branch_protection_detailed(repo, branch, desired)
                    .await?;
                if let Some(required) = desired.require_signed_commits {
                    client
                        .set_required_signatures(repo, branch, required)
                        .await?;
                }
                applied_steps.push(format!("upsert:{branch}"));
            }
            BranchProtectionPlanAction::Delete { branch } => {
                client.delete_branch_protection(repo, branch).await?;
                applied_steps.push(format!("delete:{branch}"));
            }
            BranchProtectionPlanAction::Unchanged { .. } => {}
        }
    }

    Ok(BranchProtectionApplyResult { applied_steps })
}

pub async fn verify_branch_protection_category(
    client: &Client,
    repo: &str,
    desired: &BranchProtectionCategoryV2,
) -> Result<BranchProtectionVerifyResult> {
    let actual = collect_branch_protection_category(client, repo, Some(desired)).await?;
    let plan = plan_branch_protection_category(desired, &actual)?;
    let blocked = plan
        .issues
        .iter()
        .any(|issue| issue.severity == ReconcileIssueSeverity::Blocker);
    Ok(BranchProtectionVerifyResult {
        matches: !plan.has_changes() && !blocked,
        plan,
    })
}

fn status_object(enabled: bool) -> serde_json::Value {
    serde_json::json!({
        "status": if enabled { "enabled" } else { "disabled" },
    })
}

fn codeql_state_to_manifest(state: &CodeqlDefaultSetupState) -> CodeqlDefaultSetupConfig {
    CodeqlDefaultSetupConfig {
        state: state.state.clone(),
        languages: state.languages.clone(),
        query_suite: state.query_suite.clone(),
        runner_type: state.runner_type.clone(),
        runner_label: state.runner_label.clone(),
        threat_model: state.threat_model.clone(),
    }
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

fn configuration_reference_from_attachment(
    attachment: &RepositoryCodeSecurityConfiguration,
) -> ReferencedResourceConfig {
    ReferencedResourceConfig {
        resource_type: ReferencedResourceType::CodeSecurityConfiguration,
        name: attachment.configuration.name.clone(),
    }
}

fn normalize_actor_refs(values: &[ActorReference]) -> Vec<String> {
    let mut normalized = values.iter().map(actor_reference_key).collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn actor_reference_key(actor: &ActorReference) -> String {
    match actor {
        ActorReference::OrganizationAdmin => "org-admin".to_owned(),
        ActorReference::Team { slug } => format!("team:{slug}"),
        ActorReference::User { login } => format!("user:{login}"),
        ActorReference::App { slug } => format!("app:{slug}"),
        ActorReference::Role { name } => format!("role:{name}"),
        ActorReference::Unresolved {
            actor_type,
            actor_id,
        } => {
            format!("unresolved:{actor_type}:{}", actor_id.unwrap_or_default())
        }
    }
}

fn clientless_org_hint() -> &'static str {
    "the configured organization"
}

fn collect_repository_ruleset(
    detail: RulesetDetail,
    team_by_id: &HashMap<u64, String>,
    app_by_id: &HashMap<u64, String>,
    role_by_id: &HashMap<u64, String>,
    user_by_id: &HashMap<u64, String>,
    resource_name: &str,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<RepositoryRulesetV2> {
    let conditions_json = detail
        .conditions
        .filter(|value| !value.is_null())
        .map(|value| serde_json::to_string(&value))
        .transpose()?;

    let rules = detail
        .rules
        .into_iter()
        .map(|rule| {
            Ok(RepositoryRuleConfig {
                rule_type: rule.rule_type,
                parameters_json: rule
                    .parameters
                    .map(|parameters| serde_json::to_string(&parameters))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let bypass_actors = detail
        .bypass_actors
        .into_iter()
        .filter_map(|actor| {
            let actor_type = actor.get("actor_type")?.as_str()?.to_owned();
            let actor_id = actor.get("actor_id").and_then(serde_json::Value::as_u64);
            let bypass_mode = actor
                .get("bypass_mode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("always")
                .to_owned();
            let actor = match actor_type.as_str() {
                "OrganizationAdmin" => ActorReference::OrganizationAdmin,
                "Team" => actor
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .map(|slug| ActorReference::Team {
                        slug: slug.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| team_by_id.get(&id))
                            .map(|slug| ActorReference::Team { slug: slug.clone() })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "User" => actor
                    .get("login")
                    .and_then(serde_json::Value::as_str)
                    .map(|login| ActorReference::User {
                        login: login.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| user_by_id.get(&id))
                            .map(|login| ActorReference::User {
                                login: login.clone(),
                            })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "Integration" => actor
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .map(|slug| ActorReference::App {
                        slug: slug.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| app_by_id.get(&id))
                            .map(|slug| ActorReference::App { slug: slug.clone() })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "RepositoryRole" => actor
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(|name| ActorReference::Role {
                        name: name.to_owned(),
                    })
                    .or_else(|| {
                        actor_id
                            .and_then(|id| role_by_id.get(&id))
                            .map(|name| ActorReference::Role { name: name.clone() })
                    })
                    .unwrap_or(ActorReference::Unresolved {
                        actor_type,
                        actor_id,
                    }),
                "DeployKey" if actor_id.is_none() => ActorReference::Unresolved {
                    actor_type,
                    actor_id: None,
                },
                _ => ActorReference::Unresolved {
                    actor_type,
                    actor_id,
                },
            };
            let is_stable_deploy_key = matches!(
                &actor,
                ActorReference::Unresolved {
                    actor_type,
                    actor_id: None,
                } if actor_type == "DeployKey"
            );
            if matches!(actor, ActorReference::Unresolved { .. }) && !is_stable_deploy_key {
                issues.push(blocker_issue(
                    Some(resource_name.to_owned()),
                    "rulesets-unresolved-bypass-actor",
                    format!(
                        "Ruleset {} contains a bypass actor that could not be resolved to a stable manifest identity: {}",
                        resource_name,
                        actor_reference_key(&actor)
                    ),
                ));
            }
            Some(RulesetBypassActorV2 { actor, bypass_mode })
        })
        .collect::<Vec<_>>();

    Ok(RepositoryRulesetV2 {
        name: detail.name,
        target: if detail.target.is_empty() {
            "branch".to_owned()
        } else {
            detail.target
        },
        enforcement: detail.enforcement,
        conditions_json,
        rules,
        bypass_actors,
    })
}

fn repository_role_lookup(custom_roles: &[RulesetCustomRepositoryRole]) -> HashMap<u64, String> {
    let mut roles = HashMap::from([
        (1, "read".to_owned()),
        (2, "maintain".to_owned()),
        (3, "triage".to_owned()),
        (4, "write".to_owned()),
        (5, "admin".to_owned()),
    ]);
    for role in custom_roles {
        roles.insert(role.id, role.name.clone());
    }
    roles
}

async fn repository_ruleset_to_api_json(
    client: &Client,
    ruleset: &RepositoryRulesetV2,
    role_lookup: &HashMap<u64, String>,
    app_lookup: &HashMap<String, u64>,
) -> Result<serde_json::Value> {
    let conditions = ruleset
        .conditions_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .with_context(|| format!("Invalid conditions_json for ruleset {}", ruleset.name))?
        .unwrap_or(serde_json::Value::Null);
    let rules = ruleset
        .rules
        .iter()
        .map(|rule| {
            let mut body = serde_json::json!({ "type": rule.rule_type });
            if let Some(parameters) = rule.parameters_json.as_deref() {
                body["parameters"] = serde_json::from_str(parameters).with_context(|| {
                    format!(
                        "Invalid parameters_json for {} rule in ruleset {}",
                        rule.rule_type, ruleset.name
                    )
                })?;
            }
            Ok(body)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut bypass_actors = Vec::new();
    for actor in &ruleset.bypass_actors {
        bypass_actors
            .push(resolve_ruleset_bypass_actor(client, actor, role_lookup, app_lookup).await?);
    }

    Ok(serde_json::json!({
        "name": ruleset.name,
        "target": ruleset.target,
        "enforcement": ruleset.enforcement,
        "conditions": conditions,
        "rules": rules,
        "bypass_actors": bypass_actors,
    }))
}

async fn resolve_ruleset_bypass_actor(
    client: &Client,
    actor: &RulesetBypassActorV2,
    role_lookup: &HashMap<u64, String>,
    app_lookup: &HashMap<String, u64>,
) -> Result<serde_json::Value> {
    let (actor_id, actor_type) = match &actor.actor {
        ActorReference::OrganizationAdmin => (None, "OrganizationAdmin".to_owned()),
        ActorReference::Team { slug } => (Some(client.get_team_id(slug).await?), "Team".to_owned()),
        ActorReference::User { login } => (
            Some(client.get_user_by_login(login).await?.id),
            "User".to_owned(),
        ),
        ActorReference::App { slug } => (
            Some(*app_lookup.get(slug).with_context(|| {
                format!(
                    "Installed app {slug} was not found in organization {}",
                    client.org()
                )
            })?),
            "Integration".to_owned(),
        ),
        ActorReference::Role { name } => (
            Some(resolve_repository_role_id(name, role_lookup)?),
            "RepositoryRole".to_owned(),
        ),
        ActorReference::Unresolved {
            actor_type,
            actor_id: _,
        } if actor_type == "DeployKey" => (None, "DeployKey".to_owned()),
        ActorReference::Unresolved { actor_type, .. } => anyhow::bail!(
            "Ruleset bypass actor {} could not be resolved to a supported, stable manifest identity",
            actor_type
        ),
    };

    Ok(serde_json::json!({
        "actor_id": actor_id,
        "actor_type": actor_type,
        "bypass_mode": actor.bypass_mode,
    }))
}

fn resolve_repository_role_id(name: &str, role_lookup: &HashMap<u64, String>) -> Result<u64> {
    role_lookup
        .iter()
        .find_map(|(id, current_name)| current_name.eq_ignore_ascii_case(name).then_some(*id))
        .with_context(|| format!("Repository role {name} is not known in the current organization"))
}

fn repository_ruleset_matches(left: &RepositoryRulesetV2, right: &RepositoryRulesetV2) -> bool {
    left.name == right.name
        && left.target == right.target
        && left.enforcement == right.enforcement
        && normalized_json_string(left.conditions_json.as_deref())
            == normalized_json_string(right.conditions_json.as_deref())
        && normalized_rules(left.rules.as_slice()) == normalized_rules(right.rules.as_slice())
        && normalized_bypass_actors(left.bypass_actors.as_slice())
            == normalized_bypass_actors(right.bypass_actors.as_slice())
}

fn normalized_json_string(value: Option<&str>) -> Option<String> {
    value
        .map(|value| {
            serde_json::from_str::<serde_json::Value>(value)
                .unwrap_or(serde_json::Value::String(value.to_owned()))
        })
        .and_then(|value| serde_json::to_string(&value).ok())
}

fn normalized_rules(rules: &[RepositoryRuleConfig]) -> Vec<(String, Option<String>)> {
    let mut normalized = rules
        .iter()
        .map(|rule| {
            (
                rule.rule_type.clone(),
                normalized_json_string(rule.parameters_json.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalized_bypass_actors(actors: &[RulesetBypassActorV2]) -> Vec<(String, String)> {
    let mut normalized = actors
        .iter()
        .map(|actor| (actor_reference_key(&actor.actor), actor.bypass_mode.clone()))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_ruleset_references(
    references: &[RulesetReferenceV2],
) -> Vec<(String, String, String, String, String)> {
    let mut normalized = references
        .iter()
        .map(|reference| {
            (
                reference.name.clone(),
                reference.target.clone(),
                reference.enforcement.clone(),
                reference.source_type.clone(),
                reference.source.clone(),
            )
        })
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn validate_ruleset_bypass_actors(
    ruleset: &RepositoryRulesetV2,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<()> {
    for actor in &ruleset.bypass_actors {
        match &actor.actor {
            ActorReference::Unresolved {
                actor_type,
                actor_id,
            } if actor_type == "DeployKey" && actor_id.is_none() => {}
            ActorReference::Unresolved {
                actor_type,
                actor_id,
            } => {
                issues.push(blocker_issue(
                    Some(ruleset.name.clone()),
                    "rulesets-unresolved-bypass-actor",
                    format!(
                        "Ruleset {} declares unresolved bypass actor {}{} and cannot be safely applied.",
                        ruleset.name,
                        actor_type,
                        actor_id
                            .map(|id| format!(":{id}"))
                            .unwrap_or_default()
                    ),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn ruleset_action_sort_key(action: &RulesetPlanAction) -> (u8, String) {
    match action {
        RulesetPlanAction::Create { ruleset } => (0, ruleset.name.clone()),
        RulesetPlanAction::Update { ruleset, .. } => (1, ruleset.name.clone()),
        RulesetPlanAction::Delete { name, .. } => (2, name.clone()),
        RulesetPlanAction::Unchanged { name } => (3, name.clone()),
    }
}

fn protected_branch_from_detail(
    name: &str,
    detail: &DetailedBranchProtection,
    app_slugs_by_id: &HashMap<i64, String>,
) -> ProtectedBranchConfig {
    let status_check_contexts = detail
        .required_status_checks
        .as_ref()
        .map(|checks| {
            if checks.contexts.is_empty() {
                checks
                    .checks
                    .iter()
                    .map(|check| check.context.clone())
                    .collect::<Vec<_>>()
            } else {
                checks.contexts.clone()
            }
        })
        .unwrap_or_default();
    let status_checks = detail
        .required_status_checks
        .as_ref()
        .map_or_else(Vec::new, |checks| {
            if checks.checks.is_empty() {
                checks
                    .contexts
                    .iter()
                    .map(|context| BranchStatusCheckConfigV2 {
                        context: context.clone(),
                        app_id: None,
                        app_slug: None,
                    })
                    .collect()
            } else {
                checks
                    .checks
                    .iter()
                    .map(|check| BranchStatusCheckConfigV2 {
                        context: check.context.clone(),
                        app_id: check.app_id,
                        app_slug: check
                            .app_id
                            .and_then(|id| app_slugs_by_id.get(&id).cloned()),
                    })
                    .collect()
            }
        });

    ProtectedBranchConfig {
        name: name.to_owned(),
        protection: BranchProtectionConfig {
            enabled: detail.required_pull_request_reviews.is_some(),
            required_approvals: detail
                .required_pull_request_reviews
                .as_ref()
                .map(|reviews| reviews.required_approving_review_count)
                .unwrap_or(0),
            dismiss_stale_reviews: detail
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|reviews| reviews.dismiss_stale_reviews),
            require_code_owner_reviews: detail
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|reviews| reviews.require_code_owner_reviews),
            require_status_checks: detail.required_status_checks.is_some(),
            strict_status_checks: detail
                .required_status_checks
                .as_ref()
                .is_some_and(|checks| checks.strict),
            enforce_admins: detail
                .enforce_admins
                .as_ref()
                .is_some_and(|value| value.enabled),
            required_linear_history: detail
                .required_linear_history
                .as_ref()
                .is_some_and(|value| value.enabled),
            allow_force_pushes: detail
                .allow_force_pushes
                .as_ref()
                .is_some_and(|value| value.enabled),
            allow_deletions: detail
                .allow_deletions
                .as_ref()
                .is_some_and(|value| value.enabled),
        },
        status_check_contexts,
        status_checks,
        push_restrictions: actor_set_to_references(
            detail.restrictions.as_ref().cloned().unwrap_or_default(),
        ),
        dismissal_restrictions: detail
            .required_pull_request_reviews
            .as_ref()
            .map(|reviews| actor_set_to_references(reviews.dismissal_restrictions.clone()))
            .unwrap_or_default(),
        pull_request_bypass_allowances: detail
            .required_pull_request_reviews
            .as_ref()
            .map(|reviews| actor_set_to_references(reviews.bypass_pull_request_allowances.clone()))
            .unwrap_or_default(),
        require_last_push_approval: detail
            .required_pull_request_reviews
            .as_ref()
            .and_then(|reviews| reviews.require_last_push_approval),
        block_creations: detail.block_creations.as_ref().map(|value| value.enabled),
        required_reviewers: detail
            .required_pull_request_reviews
            .as_ref()
            .and_then(|reviews| reviews.required_reviewers.clone()),
        require_conversation_resolution: detail
            .required_conversation_resolution
            .as_ref()
            .map(|value| value.enabled),
        require_signed_commits: detail
            .required_signatures
            .as_ref()
            .map(|value| value.enabled),
        lock_branch: detail.lock_branch.as_ref().map(|value| value.enabled),
        allow_fork_syncing: detail
            .allow_fork_syncing
            .as_ref()
            .map(|value| value.enabled),
    }
}

fn detailed_branch_config_from_manifest(
    branch: &ProtectedBranchConfig,
) -> DetailedBranchProtectionConfigV2 {
    DetailedBranchProtectionConfigV2 {
        protection: branch.protection.clone(),
        status_check_contexts: branch.status_check_contexts.clone(),
        status_checks: branch.status_checks.clone(),
        push_restrictions: branch.push_restrictions.clone(),
        dismissal_restrictions: branch.dismissal_restrictions.clone(),
        pull_request_bypass_allowances: branch.pull_request_bypass_allowances.clone(),
        require_last_push_approval: branch.require_last_push_approval,
        block_creations: branch.block_creations,
        required_reviewers: branch.required_reviewers.clone(),
        require_conversation_resolution: branch.require_conversation_resolution,
        require_signed_commits: branch.require_signed_commits,
        lock_branch: branch.lock_branch,
        allow_fork_syncing: branch.allow_fork_syncing,
    }
}

fn actor_set_to_references(actors: ActorSet) -> Vec<ActorReference> {
    let mut references = actors
        .users
        .into_iter()
        .map(|user| ActorReference::User { login: user.login })
        .chain(
            actors
                .teams
                .into_iter()
                .map(|team| ActorReference::Team { slug: team.slug }),
        )
        .chain(
            actors
                .apps
                .into_iter()
                .map(|app| ActorReference::App { slug: app.slug }),
        )
        .collect::<Vec<_>>();
    references.sort_by_key(actor_reference_key);
    references
}

fn desired_branch_protection_from_default(
    branch_name: &str,
    config: &BranchProtectionConfig,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    let preserved = existing.map(|branch| &branch.manifest);
    desired_branch_protection_from_parts(
        branch_name,
        config,
        preserved
            .map(|branch| branch.status_check_contexts.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.status_checks.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.push_restrictions.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.dismissal_restrictions.as_slice())
            .unwrap_or(&[]),
        preserved
            .map(|branch| branch.pull_request_bypass_allowances.as_slice())
            .unwrap_or(&[]),
        existing.and_then(|branch| {
            branch
                .raw
                .required_pull_request_reviews
                .as_ref()
                .and_then(|reviews| reviews.require_last_push_approval)
        }),
        existing
            .and_then(|branch| branch.raw.block_creations.as_ref())
            .map(|value| value.enabled),
        existing
            .and_then(|branch| branch.raw.required_pull_request_reviews.as_ref())
            .and_then(|reviews| reviews.required_reviewers.clone()),
        preserved.and_then(|branch| branch.require_conversation_resolution),
        preserved.and_then(|branch| branch.require_signed_commits),
        preserved.and_then(|branch| branch.lock_branch),
        preserved.and_then(|branch| branch.allow_fork_syncing),
        existing,
        app_ids_by_slug,
        issues,
    )
}

fn desired_branch_protection_from_detailed_manifest(
    branch_name: &str,
    config: &DetailedBranchProtectionConfigV2,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    desired_branch_protection_from_parts(
        branch_name,
        &config.protection,
        config.status_check_contexts.as_slice(),
        config.status_checks.as_slice(),
        config.push_restrictions.as_slice(),
        config.dismissal_restrictions.as_slice(),
        config.pull_request_bypass_allowances.as_slice(),
        config.require_last_push_approval,
        config.block_creations,
        config.required_reviewers.clone(),
        config.require_conversation_resolution,
        config.require_signed_commits,
        config.lock_branch,
        config.allow_fork_syncing,
        existing,
        app_ids_by_slug,
        issues,
    )
}

fn desired_branch_protection_from_manifest(
    protected_branch: &ProtectedBranchConfig,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    desired_branch_protection_from_parts(
        &protected_branch.name,
        &protected_branch.protection,
        protected_branch.status_check_contexts.as_slice(),
        protected_branch.status_checks.as_slice(),
        protected_branch.push_restrictions.as_slice(),
        protected_branch.dismissal_restrictions.as_slice(),
        protected_branch.pull_request_bypass_allowances.as_slice(),
        protected_branch.require_last_push_approval,
        protected_branch.block_creations,
        protected_branch.required_reviewers.clone(),
        protected_branch.require_conversation_resolution,
        protected_branch.require_signed_commits,
        protected_branch.lock_branch,
        protected_branch.allow_fork_syncing,
        existing,
        app_ids_by_slug,
        issues,
    )
}

#[allow(clippy::too_many_arguments)]
fn desired_branch_protection_from_parts(
    branch_name: &str,
    config: &BranchProtectionConfig,
    status_check_contexts: &[String],
    status_checks_config: &[BranchStatusCheckConfigV2],
    push_restrictions: &[ActorReference],
    dismissal_restrictions: &[ActorReference],
    pull_request_bypass_allowances: &[ActorReference],
    require_last_push_approval: Option<bool>,
    block_creations: Option<bool>,
    required_reviewers: Option<serde_json::Value>,
    require_conversation_resolution: Option<bool>,
    require_signed_commits: Option<bool>,
    lock_branch: Option<bool>,
    allow_fork_syncing: Option<bool>,
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<DesiredBranchProtection> {
    let push_restrictions = actor_refs_to_actor_set(branch_name, push_restrictions, issues)?;
    let dismissal_restrictions =
        actor_refs_to_actor_set(branch_name, dismissal_restrictions, issues)?;
    let pull_request_bypass_allowances =
        actor_refs_to_actor_set(branch_name, pull_request_bypass_allowances, issues)?;
    let desired_status_checks = desired_status_checks(
        branch_name,
        status_check_contexts,
        status_checks_config,
        existing,
        app_ids_by_slug,
        issues,
    )?;

    Ok(DesiredBranchProtection {
        required_pull_request_reviews: config.enabled,
        required_approving_review_count: config.required_approvals,
        dismiss_stale_reviews: config.dismiss_stale_reviews,
        require_code_owner_reviews: config.require_code_owner_reviews,
        require_last_push_approval,
        required_status_checks: config.require_status_checks,
        strict_status_checks: config.strict_status_checks,
        status_check_contexts: status_check_contexts.to_vec(),
        status_checks: desired_status_checks,
        push_restrictions,
        dismissal_restrictions,
        pull_request_bypass_allowances,
        enforce_admins: config.enforce_admins,
        required_linear_history: config.required_linear_history,
        allow_force_pushes: config.allow_force_pushes,
        allow_deletions: config.allow_deletions,
        block_creations,
        require_conversation_resolution,
        require_signed_commits,
        lock_branch,
        allow_fork_syncing,
        required_reviewers,
    })
}

fn desired_status_checks(
    branch_name: &str,
    status_check_contexts: &[String],
    status_checks_config: &[BranchStatusCheckConfigV2],
    existing: Option<&ActualProtectedBranch>,
    app_ids_by_slug: &HashMap<String, i64>,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<Vec<StatusCheckRequirement>> {
    if !status_checks_config.is_empty() {
        return status_checks_config
            .iter()
            .map(|check| {
                let app_id = match (&check.app_slug, check.app_id) {
                    (Some(slug), _) => Some(*app_ids_by_slug.get(slug).with_context(|| {
                        format!(
                            "Status check {} on {} references unknown GitHub App slug {}",
                            check.context, branch_name, slug
                        )
                    })?),
                    (None, Some(app_id)) => {
                        issues.push(blocker_issue(
                            Some(branch_name.to_owned()),
                            "branch-protection-unresolved-status-check-app",
                            format!(
                                "Status check {} on {} is pinned to app_id {} without a stable app_slug; refusing to apply unresolved app bindings.",
                                check.context, branch_name, app_id
                            ),
                        ));
                        Some(app_id)
                    }
                    (None, None) => None,
                };
                Ok(StatusCheckRequirement {
                    context: check.context.clone(),
                    app_id,
                })
            })
            .collect();
    }

    if let Some(existing) = existing {
        let actual_contexts = existing
            .status_checks
            .iter()
            .map(|check| check.context.clone())
            .collect::<Vec<_>>();
        if actual_contexts == status_check_contexts {
            return Ok(existing.status_checks.clone());
        }
    }

    Ok(status_check_contexts
        .iter()
        .map(|context| StatusCheckRequirement {
            context: context.clone(),
            app_id: None,
        })
        .collect())
}

fn reviewer_options_from_api(
    options: &crate::github::security::DelegatedBypassOptions,
    team_ids_by_slug: &HashMap<String, u64>,
    repository_role_ids_by_name: &HashMap<String, u64>,
    repo: &str,
    issues: &mut Vec<ReconcileIssue>,
) -> Result<SecurityReviewerOptionsConfigV2> {
    let team_slugs_by_id = team_ids_by_slug
        .iter()
        .map(|(slug, id)| (*id, slug.clone()))
        .collect::<HashMap<_, _>>();
    let role_names_by_id = repository_role_ids_by_name
        .iter()
        .map(|(name, id)| (*id, name.clone()))
        .collect::<HashMap<_, _>>();

    let reviewers = options
        .reviewers
        .iter()
        .map(|reviewer| {
            let actor = match reviewer.reviewer_type.as_str() {
                "Team" | "TEAM" => team_slugs_by_id
                    .get(&reviewer.reviewer_id)
                    .cloned()
                    .map(|slug| ActorReference::Team { slug })
                    .unwrap_or_else(|| {
                        issues.push(warning_issue(
                            Some(repo.to_owned()),
                            "security-reviewer-team-unresolved",
                            format!(
                                "Could not resolve delegated reviewer team id {} for {repo}; preserving it as unresolved.",
                                reviewer.reviewer_id
                            ),
                        ));
                        ActorReference::Unresolved {
                            actor_type: reviewer.reviewer_type.clone(),
                            actor_id: Some(reviewer.reviewer_id),
                        }
                    }),
                "RepositoryRole" | "ROLE" => role_names_by_id
                    .get(&reviewer.reviewer_id)
                    .cloned()
                    .map(|name| ActorReference::Role { name })
                    .unwrap_or_else(|| {
                        issues.push(warning_issue(
                            Some(repo.to_owned()),
                            "security-reviewer-role-unresolved",
                            format!(
                                "Could not resolve delegated reviewer repository role id {} for {repo}; preserving it as unresolved.",
                                reviewer.reviewer_id
                            ),
                        ));
                        ActorReference::Unresolved {
                            actor_type: reviewer.reviewer_type.clone(),
                            actor_id: Some(reviewer.reviewer_id),
                        }
                    }),
                _ => {
                    issues.push(warning_issue(
                        Some(repo.to_owned()),
                        "security-reviewer-actor-unsupported",
                        format!(
                            "Delegated reviewer type {} on {repo} is not supported for stable reconciliation; preserving it as unresolved.",
                            reviewer.reviewer_type
                        ),
                    ));
                    ActorReference::Unresolved {
                        actor_type: reviewer.reviewer_type.clone(),
                        actor_id: Some(reviewer.reviewer_id),
                    }
                }
            };
            Ok(SecurityReviewerConfigV2 {
                actor,
                mode: reviewer.mode.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(SecurityReviewerOptionsConfigV2 { reviewers })
}

fn security_reviewer_options_to_api_json(
    options: &SecurityReviewerOptionsConfigV2,
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

fn actor_refs_to_actor_set(
    branch_name: &str,
    actors: &[ActorReference],
    issues: &mut Vec<ReconcileIssue>,
) -> Result<ActorSet> {
    let mut set = ActorSet::default();
    for actor in actors {
        match actor {
            ActorReference::User { login } => set.users.push(UserActor {
                login: login.clone(),
            }),
            ActorReference::Team { slug } => set.teams.push(TeamActor { slug: slug.clone() }),
            ActorReference::App { slug } => set.apps.push(AppActor { slug: slug.clone() }),
            _ => issues.push(blocker_issue(
                Some(branch_name.to_owned()),
                "branch-protection-unsupported-actor",
                format!(
                    "Legacy branch protection only supports user, team, and app actors; {} is not supported",
                    actor_reference_key(actor)
                ),
            )),
        }
    }
    set.users
        .sort_by(|left, right| left.login.cmp(&right.login));
    set.teams.sort_by(|left, right| left.slug.cmp(&right.slug));
    set.apps.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(set)
}

fn protected_branch_matches(
    actual: &ProtectedBranchConfig,
    desired: &DesiredBranchProtection,
) -> bool {
    actual.protection.enabled == desired.required_pull_request_reviews
        && actual.protection.required_approvals == desired.required_approving_review_count
        && actual.protection.dismiss_stale_reviews == desired.dismiss_stale_reviews
        && actual.protection.require_code_owner_reviews == desired.require_code_owner_reviews
        && actual.protection.require_status_checks == desired.required_status_checks
        && actual.protection.strict_status_checks == desired.strict_status_checks
        && actual.protection.enforce_admins == desired.enforce_admins
        && actual.protection.required_linear_history == desired.required_linear_history
        && actual.protection.allow_force_pushes == desired.allow_force_pushes
        && actual.protection.allow_deletions == desired.allow_deletions
        && normalize_strings(actual.status_check_contexts.as_slice())
            == normalize_strings(desired.status_check_contexts.as_slice())
        && normalize_branch_status_checks(actual.status_checks.as_slice())
            == normalize_status_check_requirements(desired.status_checks.as_slice())
        && normalize_actor_refs(actual.push_restrictions.as_slice())
            == normalize_actor_set(&desired.push_restrictions)
        && normalize_actor_refs(actual.dismissal_restrictions.as_slice())
            == normalize_actor_set(&desired.dismissal_restrictions)
        && normalize_actor_refs(actual.pull_request_bypass_allowances.as_slice())
            == normalize_actor_set(&desired.pull_request_bypass_allowances)
        && actual.require_last_push_approval == desired.require_last_push_approval
        && actual.block_creations == desired.block_creations
        && normalize_optional_json(actual.required_reviewers.as_ref())
            == normalize_optional_json(desired.required_reviewers.as_ref())
        && actual.require_conversation_resolution == desired.require_conversation_resolution
        && actual.require_signed_commits == desired.require_signed_commits
        && actual.lock_branch == desired.lock_branch
        && actual.allow_fork_syncing == desired.allow_fork_syncing
}

fn normalize_actor_set(set: &ActorSet) -> Vec<String> {
    let mut normalized = set
        .users
        .iter()
        .map(|user| format!("user:{}", user.login))
        .chain(set.teams.iter().map(|team| format!("team:{}", team.slug)))
        .chain(set.apps.iter().map(|app| format!("app:{}", app.slug)))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_branch_status_checks(
    values: &[BranchStatusCheckConfigV2],
) -> Vec<(String, Option<i64>)> {
    let mut normalized = values
        .iter()
        .map(|check| (check.context.clone(), check.app_id))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_status_check_requirements(
    values: &[StatusCheckRequirement],
) -> Vec<(String, Option<i64>)> {
    let mut normalized = values
        .iter()
        .map(|check| (check.context.clone(), check.app_id))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_optional_json(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(|value| serde_json::to_string(value).ok())
}

fn normalize_strings(values: &[String]) -> Vec<String> {
    let mut normalized = values.to_vec();
    normalized.sort();
    normalized
}

fn bool_field(value: &Option<crate::github::security::SecurityFeatureStatus>) -> Option<bool> {
    value.as_ref().map(|status| status.status == "enabled")
}

fn branch_action_sort_key(action: &BranchProtectionPlanAction) -> (u8, String) {
    match action {
        BranchProtectionPlanAction::Upsert { branch, .. } => (0, branch.clone()),
        BranchProtectionPlanAction::Delete { branch } => (1, branch.clone()),
        BranchProtectionPlanAction::Unchanged { branch } => (2, branch.clone()),
    }
}

fn collected_entry(category: ManifestCategoryName, endpoint: &str) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::Collected,
        reason: None,
        required_permission: None,
    }
}

fn permission_denied_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    reason: String,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::PermissionDenied,
        reason: Some(reason),
        required_permission: None,
    }
}

fn not_applicable_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    reason: String,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::NotApplicable,
        reason: Some(reason),
        required_permission: None,
    }
}

fn unavailable_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    reason: String,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::Unavailable,
        reason: Some(reason),
        required_permission: None,
    }
}

fn warning_issue(resource: Option<String>, code: &'static str, message: String) -> ReconcileIssue {
    ReconcileIssue {
        resource,
        code,
        severity: ReconcileIssueSeverity::Warning,
        message,
    }
}

fn blocker_issue(resource: Option<String>, code: &'static str, message: String) -> ReconcileIssue {
    ReconcileIssue {
        resource,
        code,
        severity: ReconcileIssueSeverity::Blocker,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn ruleset_match_is_order_insensitive() {
        let left = RepositoryRulesetV2 {
            name: "main".to_owned(),
            target: "branch".to_owned(),
            enforcement: "active".to_owned(),
            conditions_json: Some(
                r#"{"ref_name":{"include":["~DEFAULT_BRANCH"],"exclude":[]}}"#.to_owned(),
            ),
            rules: vec![
                RepositoryRuleConfig {
                    rule_type: "deletion".to_owned(),
                    parameters_json: None,
                },
                RepositoryRuleConfig {
                    rule_type: "required_signatures".to_owned(),
                    parameters_json: None,
                },
            ],
            bypass_actors: vec![RulesetBypassActorV2 {
                actor: ActorReference::Team {
                    slug: "platform".to_owned(),
                },
                bypass_mode: "always".to_owned(),
            }],
        };
        let mut right = left.clone();
        right.rules.reverse();

        assert!(repository_ruleset_matches(&left, &right));
    }

    #[test]
    fn branch_matches_compare_actor_sets_and_contexts() {
        let actual = ProtectedBranchConfig {
            name: "main".to_owned(),
            protection: BranchProtectionConfig {
                enabled: true,
                required_approvals: 2,
                dismiss_stale_reviews: true,
                require_code_owner_reviews: true,
                require_status_checks: true,
                strict_status_checks: true,
                enforce_admins: true,
                required_linear_history: true,
                allow_force_pushes: false,
                allow_deletions: false,
            },
            status_check_contexts: vec!["ci".to_owned(), "lint".to_owned()],
            status_checks: Vec::new(),
            push_restrictions: vec![ActorReference::Team {
                slug: "platform".to_owned(),
            }],
            dismissal_restrictions: vec![ActorReference::User {
                login: "alice".to_owned(),
            }],
            pull_request_bypass_allowances: vec![ActorReference::App {
                slug: "release-bot".to_owned(),
            }],
            require_last_push_approval: None,
            block_creations: None,
            required_reviewers: None,
            require_conversation_resolution: Some(true),
            require_signed_commits: Some(true),
            lock_branch: Some(false),
            allow_fork_syncing: Some(false),
        };
        let desired = DesiredBranchProtection {
            required_pull_request_reviews: true,
            required_approving_review_count: 2,
            dismiss_stale_reviews: true,
            require_code_owner_reviews: true,
            require_last_push_approval: None,
            required_status_checks: true,
            strict_status_checks: true,
            status_check_contexts: vec!["lint".to_owned(), "ci".to_owned()],
            status_checks: Vec::new(),
            push_restrictions: ActorSet {
                users: Vec::new(),
                teams: vec![TeamActor {
                    slug: "platform".to_owned(),
                }],
                apps: Vec::new(),
            },
            dismissal_restrictions: ActorSet {
                users: vec![UserActor {
                    login: "alice".to_owned(),
                }],
                teams: Vec::new(),
                apps: Vec::new(),
            },
            pull_request_bypass_allowances: ActorSet {
                users: Vec::new(),
                teams: Vec::new(),
                apps: vec![AppActor {
                    slug: "release-bot".to_owned(),
                }],
            },
            enforce_admins: true,
            required_linear_history: true,
            allow_force_pushes: false,
            allow_deletions: false,
            block_creations: None,
            require_conversation_resolution: Some(true),
            require_signed_commits: Some(true),
            lock_branch: Some(false),
            allow_fork_syncing: Some(false),
            required_reviewers: None,
        };

        assert!(protected_branch_matches(&actual, &desired));
    }

    #[tokio::test]
    async fn resolve_ruleset_bypass_actor_preserves_user_deploy_key_org_admin_and_base_role() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/alice"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": 2,
                "login": "alice"
            })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let role_lookup = repository_role_lookup(&[]);
        let app_lookup = HashMap::new();

        let user = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActorV2 {
                actor: ActorReference::User {
                    login: "alice".to_owned(),
                },
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            user,
            json!({
                "actor_id": 2,
                "actor_type": "User",
                "bypass_mode": "always"
            })
        );

        let org_admin = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActorV2 {
                actor: ActorReference::OrganizationAdmin,
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            org_admin,
            json!({
                "actor_id": null,
                "actor_type": "OrganizationAdmin",
                "bypass_mode": "always"
            })
        );

        let base_role = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActorV2 {
                actor: ActorReference::Role {
                    name: "admin".to_owned(),
                },
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            base_role,
            json!({
                "actor_id": 5,
                "actor_type": "RepositoryRole",
                "bypass_mode": "always"
            })
        );

        let deploy_key = resolve_ruleset_bypass_actor(
            &client,
            &RulesetBypassActorV2 {
                actor: ActorReference::Unresolved {
                    actor_type: "DeployKey".to_owned(),
                    actor_id: None,
                },
                bypass_mode: "always".to_owned(),
            },
            &role_lookup,
            &app_lookup,
        )
        .await
        .unwrap();
        assert_eq!(
            deploy_key,
            json!({
                "actor_id": null,
                "actor_type": "DeployKey",
                "bypass_mode": "always"
            })
        );
    }
}
