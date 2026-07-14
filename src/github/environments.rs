//! Repository environment APIs.
//!
//! Endpoints verified against the GitHub REST API reference
//! (`X-GitHub-Api-Version: 2022-11-28`) for `repos/{owner}/{repo}/environments`,
//! `.../deployment-branch-policies`, and `.../deployment_protection_rules`.
//! `protection_rules` item shapes for `required_reviewers` and `branch_policy`
//! were additionally confirmed against live public-repository API responses,
//! since GitHub's reference docs only describe the field as "array of object".

use anyhow::{Context, Result};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use super::Client;
use super::actions::{
    ReadOutcome, WriteOutcome, classify_read, write_delete, write_empty, write_json,
};
use super::response;

/// Percent-encode a single path segment (e.g. an environment name), which may
/// legally contain characters such as `/`, spaces, or other symbols that must
/// not be interpreted as path separators or otherwise misparsed by the API.
pub(crate) fn encode_path_segment(segment: &str) -> String {
    utf8_percent_encode(segment, NON_ALPHANUMERIC).to_string()
}

/// A reviewer entry on a `required_reviewers` protection rule.
#[derive(Debug, Clone, Deserialize)]
pub struct ProtectionRuleReviewer {
    #[serde(rename = "type")]
    pub reviewer_type: String,
    pub reviewer: ReviewerActor,
}

/// The actor (user or team) referenced by a protection rule reviewer entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewerActor {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
}

/// A single environment protection rule, as observed in
/// `GET /repos/{owner}/{repo}/environments` responses. GitHub tags each item
/// with a `type` discriminator; unrecognized types are preserved for
/// forward-compatibility rather than causing a parse failure.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtectionRule {
    RequiredReviewers {
        #[serde(default)]
        prevent_self_review: bool,
        #[serde(default)]
        reviewers: Vec<ProtectionRuleReviewer>,
    },
    WaitTimer {
        #[serde(default)]
        wait_timer: u32,
    },
    BranchPolicy {},
    #[serde(other)]
    Unknown,
}

/// `deployment_branch_policy` on an environment: whether branch/tag policy
/// restrictions apply, and whether they use custom patterns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DeploymentBranchPolicySummary {
    pub protected_branches: bool,
    pub custom_branch_policies: bool,
}

/// A repository deployment environment, as returned by the environments list
/// and get-single endpoints. GitHub does not surface `wait_timer`,
/// `prevent_self_review`, or `reviewers` as top-level fields on this object;
/// they must be extracted from `protection_rules` (see
/// [`Environment::wait_timer_minutes`] and [`Environment::required_reviewers`]).
#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub name: String,
    #[serde(default)]
    pub deployment_branch_policy: Option<DeploymentBranchPolicySummary>,
    #[serde(default)]
    pub protection_rules: Vec<ProtectionRule>,
}

impl Environment {
    /// The wait-timer duration in minutes, extracted from a `WaitTimer`
    /// protection rule if one is present.
    pub fn wait_timer_minutes(&self) -> Option<u32> {
        self.protection_rules.iter().find_map(|rule| match rule {
            ProtectionRule::WaitTimer { wait_timer } => Some(*wait_timer),
            _ => None,
        })
    }

    /// The `prevent_self_review` flag and reviewer list, extracted from a
    /// `RequiredReviewers` protection rule if one is present. Returns `None`
    /// when there is no such rule (no reviewers configured), which callers
    /// should distinguish from "reviewers configured but empty".
    pub fn required_reviewers(&self) -> Option<(bool, &[ProtectionRuleReviewer])> {
        self.protection_rules.iter().find_map(|rule| match rule {
            ProtectionRule::RequiredReviewers {
                prevent_self_review,
                reviewers,
            } => Some((*prevent_self_review, reviewers.as_slice())),
            _ => None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct EnvironmentsResponse {
    environments: Vec<Environment>,
}

/// A single deployment branch or tag policy pattern on an environment.
#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentBranchPolicy {
    pub id: u64,
    pub name: String,
    #[serde(rename = "type", default = "default_branch_policy_type")]
    pub policy_type: String,
}

fn default_branch_policy_type() -> String {
    "branch".to_owned()
}

#[derive(Debug, Deserialize)]
struct DeploymentBranchPoliciesResponse {
    branch_policies: Vec<DeploymentBranchPolicy>,
}

/// A custom deployment protection rule (third-party app) enabled on an environment.
#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentProtectionRule {
    pub id: u64,
    pub enabled: bool,
    pub app: DeploymentProtectionRuleApp,
}

/// The GitHub App backing a custom deployment protection rule.
#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentProtectionRuleApp {
    pub id: u64,
    pub slug: String,
}

#[derive(Debug, Deserialize)]
struct DeploymentProtectionRulesResponse {
    custom_deployment_protection_rules: Vec<DeploymentProtectionRule>,
}

/// An available (installed) GitHub App integration that can back a custom
/// deployment protection rule, as returned by the "list available apps" endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct AvailableDeploymentProtectionRuleApp {
    pub id: u64,
    pub slug: String,
}

#[derive(Debug, Deserialize)]
struct AvailableDeploymentProtectionRuleAppsResponse {
    available_custom_deployment_protection_rule_integrations:
        Vec<AvailableDeploymentProtectionRuleApp>,
}

/// Desired reviewer input for creating/updating an environment.
#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentReviewerInput {
    #[serde(rename = "type")]
    pub reviewer_type: &'static str,
    pub id: u64,
}

/// Desired state for `PUT /repos/{owner}/{repo}/environments/{environment_name}`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EnvironmentUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_timer: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prevent_self_review: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewers: Option<Vec<EnvironmentReviewerInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_branch_policy: Option<DeploymentBranchPolicySummary>,
}

impl Client {
    // ---- Environments ----

    /// `GET /repos/{owner}/{repo}/environments`, paginated (wrapped response,
    /// not a raw array, so this cannot use the generic `collect_paginated` helper).
    pub async fn list_environments(&self, repo: &str) -> Result<Vec<Environment>> {
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/environments?per_page=30&page={page}",
                self.org()
            );
            let body: EnvironmentsResponse =
                response::expect_json(self.get(&path).await?, "GET", &path)
                    .await
                    .context("Failed to parse environments response")?;
            let count = body.environments.len();
            items.extend(body.environments);
            if count < 30 {
                break;
            }
            page += 1;
        }
        Ok(items)
    }

    /// As [`Client::list_environments`], classified: a 403/404/422 on the
    /// first page is reported as a [`ReadOutcome`] instead of failing.
    pub async fn list_environments_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<Environment>>> {
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/environments?per_page=30&page={page}",
                self.org()
            );
            let response = self.get(&path).await?;
            if page == 1 {
                let body: EnvironmentsResponse =
                    match classify_read(response, "GET", &path, false).await? {
                        ReadOutcome::Available(body) => body,
                        ReadOutcome::NotApplicable(reason) => {
                            return Ok(ReadOutcome::NotApplicable(reason));
                        }
                        ReadOutcome::PermissionDenied(reason) => {
                            return Ok(ReadOutcome::PermissionDenied(reason));
                        }
                        ReadOutcome::Unavailable(reason) => {
                            return Ok(ReadOutcome::Unavailable(reason));
                        }
                    };
                let count = body.environments.len();
                items.extend(body.environments);
                if count < 30 {
                    break;
                }
            } else {
                let body: EnvironmentsResponse = response::expect_json(response, "GET", &path)
                    .await
                    .context("Failed to parse environments response")?;
                let count = body.environments.len();
                items.extend(body.environments);
                if count < 30 {
                    break;
                }
            }
            page += 1;
        }
        Ok(ReadOutcome::Available(items))
    }

    /// `GET /repos/{owner}/{repo}/environments/{environment_name}`.
    /// Returns `Ok(None)` on 404 (no such environment).
    pub async fn get_environment(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<Option<Environment>> {
        let env = encode_path_segment(environment_name);
        let path = format!("/repos/{}/{repo}/environments/{env}", self.org());
        response::optional_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse environment response")
    }

    /// `PUT /repos/{owner}/{repo}/environments/{environment_name}`. Creates the
    /// environment if it does not already exist. May respond `422` if the
    /// requested configuration is invalid (e.g. reviewers exceed the allowed
    /// count), which callers should surface as a blocked action.
    pub async fn put_environment(
        &self,
        repo: &str,
        environment_name: &str,
        update: &EnvironmentUpdate,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!("/repos/{}/{repo}/environments/{env}", self.org());
        write_empty(self.put_json(&path, update).await?, "PUT", &path).await
    }

    /// `DELETE /repos/{owner}/{repo}/environments/{environment_name}`.
    pub async fn delete_environment(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!("/repos/{}/{repo}/environments/{env}", self.org());
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    // ---- Deployment branch/tag policies ----

    /// `GET /repos/{owner}/{repo}/environments/{environment_name}/deployment-branch-policies`, paginated.
    pub async fn list_deployment_branch_policies(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<Vec<DeploymentBranchPolicy>> {
        let env = encode_path_segment(environment_name);
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/environments/{env}/deployment-branch-policies?per_page=30&page={page}",
                self.org()
            );
            let body: DeploymentBranchPoliciesResponse =
                response::expect_json(self.get(&path).await?, "GET", &path)
                    .await
                    .context("Failed to parse deployment branch policies response")?;
            let count = body.branch_policies.len();
            items.extend(body.branch_policies);
            if count < 30 {
                break;
            }
            page += 1;
        }
        Ok(items)
    }

    /// As [`Client::list_deployment_branch_policies`], classified.
    pub async fn list_deployment_branch_policies_checked(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<ReadOutcome<Vec<DeploymentBranchPolicy>>> {
        let env = encode_path_segment(environment_name);
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/environments/{env}/deployment-branch-policies?per_page=30&page={page}",
                self.org()
            );
            let response = self.get(&path).await?;
            if page == 1 {
                let body: DeploymentBranchPoliciesResponse =
                    match classify_read(response, "GET", &path, false).await? {
                        ReadOutcome::Available(body) => body,
                        ReadOutcome::NotApplicable(reason) => {
                            return Ok(ReadOutcome::NotApplicable(reason));
                        }
                        ReadOutcome::PermissionDenied(reason) => {
                            return Ok(ReadOutcome::PermissionDenied(reason));
                        }
                        ReadOutcome::Unavailable(reason) => {
                            return Ok(ReadOutcome::Unavailable(reason));
                        }
                    };
                let count = body.branch_policies.len();
                items.extend(body.branch_policies);
                if count < 30 {
                    break;
                }
            } else {
                let body: DeploymentBranchPoliciesResponse =
                    response::expect_json(response, "GET", &path)
                        .await
                        .context("Failed to parse deployment branch policies response")?;
                let count = body.branch_policies.len();
                items.extend(body.branch_policies);
                if count < 30 {
                    break;
                }
            }
            page += 1;
        }
        Ok(ReadOutcome::Available(items))
    }

    /// `POST /repos/{owner}/{repo}/environments/{environment_name}/deployment-branch-policies`.
    ///
    /// Responds `200` (not `201`) on success. Callers should list existing
    /// policies first and skip creating patterns that already exist: GitHub
    /// responds `303 See Other` for a duplicate pattern, and since the
    /// underlying HTTP client follows redirects automatically, relying on
    /// that response code is unreliable.
    pub async fn create_deployment_branch_policy(
        &self,
        repo: &str,
        environment_name: &str,
        name: &str,
        policy_type: &str,
    ) -> Result<WriteOutcome<DeploymentBranchPolicy>> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/deployment-branch-policies",
            self.org()
        );
        write_json(
            self.post_json(
                &path,
                &serde_json::json!({ "name": name, "type": policy_type }),
            )
            .await?,
            "POST",
            &path,
        )
        .await
    }

    /// `DELETE /repos/{owner}/{repo}/environments/{environment_name}/deployment-branch-policies/{branch_policy_id}`.
    pub async fn delete_deployment_branch_policy(
        &self,
        repo: &str,
        environment_name: &str,
        branch_policy_id: u64,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/deployment-branch-policies/{branch_policy_id}",
            self.org()
        );
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    // ---- Deployment protection rules (custom apps) ----

    /// `GET /repos/{owner}/{repo}/environments/{environment_name}/deployment_protection_rules`, paginated.
    pub async fn list_deployment_protection_rules(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<Vec<DeploymentProtectionRule>> {
        let env = encode_path_segment(environment_name);
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/environments/{env}/deployment_protection_rules?per_page=30&page={page}",
                self.org()
            );
            let body: DeploymentProtectionRulesResponse =
                response::expect_json(self.get(&path).await?, "GET", &path)
                    .await
                    .context("Failed to parse deployment protection rules response")?;
            let count = body.custom_deployment_protection_rules.len();
            items.extend(body.custom_deployment_protection_rules);
            if count < 30 {
                break;
            }
            page += 1;
        }
        Ok(items)
    }

    /// As [`Client::list_deployment_protection_rules`], classified.
    pub async fn list_deployment_protection_rules_checked(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<ReadOutcome<Vec<DeploymentProtectionRule>>> {
        let env = encode_path_segment(environment_name);
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/environments/{env}/deployment_protection_rules?per_page=30&page={page}",
                self.org()
            );
            let response = self.get(&path).await?;
            if page == 1 {
                let body: DeploymentProtectionRulesResponse =
                    match classify_read(response, "GET", &path, false).await? {
                        ReadOutcome::Available(body) => body,
                        ReadOutcome::NotApplicable(reason) => {
                            return Ok(ReadOutcome::NotApplicable(reason));
                        }
                        ReadOutcome::PermissionDenied(reason) => {
                            return Ok(ReadOutcome::PermissionDenied(reason));
                        }
                        ReadOutcome::Unavailable(reason) => {
                            return Ok(ReadOutcome::Unavailable(reason));
                        }
                    };
                let count = body.custom_deployment_protection_rules.len();
                items.extend(body.custom_deployment_protection_rules);
                if count < 30 {
                    break;
                }
            } else {
                let body: DeploymentProtectionRulesResponse =
                    response::expect_json(response, "GET", &path)
                        .await
                        .context("Failed to parse deployment protection rules response")?;
                let count = body.custom_deployment_protection_rules.len();
                items.extend(body.custom_deployment_protection_rules);
                if count < 30 {
                    break;
                }
            }
            page += 1;
        }
        Ok(ReadOutcome::Available(items))
    }

    /// `GET /repos/{owner}/{repo}/environments/{environment_name}/deployment_protection_rules/apps`,
    /// paginated. Used to resolve an app's `slug` (as stored in the manifest)
    /// to the numeric `integration_id` required to enable a protection rule.
    pub async fn list_available_deployment_protection_rule_apps(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<Vec<AvailableDeploymentProtectionRuleApp>> {
        let env = encode_path_segment(environment_name);
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/environments/{env}/deployment_protection_rules/apps?per_page=30&page={page}",
                self.org()
            );
            let body: AvailableDeploymentProtectionRuleAppsResponse =
                response::expect_json(self.get(&path).await?, "GET", &path)
                    .await
                    .context(
                        "Failed to parse available deployment protection rule apps response",
                    )?;
            let count = body
                .available_custom_deployment_protection_rule_integrations
                .len();
            items.extend(body.available_custom_deployment_protection_rule_integrations);
            if count < 30 {
                break;
            }
            page += 1;
        }
        Ok(items)
    }

    /// `POST /repos/{owner}/{repo}/environments/{environment_name}/deployment_protection_rules`.
    /// `integration_id` must be resolved from
    /// [`Client::list_available_deployment_protection_rule_apps`] by matching the desired app's slug.
    pub async fn enable_deployment_protection_rule(
        &self,
        repo: &str,
        environment_name: &str,
        integration_id: u64,
    ) -> Result<WriteOutcome<DeploymentProtectionRule>> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/deployment_protection_rules",
            self.org()
        );
        write_json(
            self.post_json(
                &path,
                &serde_json::json!({ "integration_id": integration_id }),
            )
            .await?,
            "POST",
            &path,
        )
        .await
    }

    /// `DELETE /repos/{owner}/{repo}/environments/{environment_name}/deployment_protection_rules/{protection_rule_id}`.
    pub async fn disable_deployment_protection_rule(
        &self,
        repo: &str,
        environment_name: &str,
        protection_rule_id: u64,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/deployment_protection_rules/{protection_rule_id}",
            self.org()
        );
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }
}

#[cfg(test)]
mod tests {
    use super::encode_path_segment;

    #[test]
    fn encodes_slashes_and_spaces_in_environment_names() {
        // `NON_ALPHANUMERIC` percent-encodes every non-alphanumeric byte,
        // including `-`; this is intentionally conservative (over-encoding
        // is always safe and GitHub decodes it correctly) rather than
        // maintaining a bespoke "safe" character allowlist.
        assert_eq!(
            encode_path_segment("staging/eu-west"),
            "staging%2Feu%2Dwest"
        );
        assert_eq!(encode_path_segment("prod deploy"), "prod%20deploy");
    }

    #[test]
    fn leaves_alphanumeric_names_untouched() {
        assert_eq!(encode_path_segment("production"), "production");
        assert_eq!(encode_path_segment("staging2"), "staging2");
    }
}
