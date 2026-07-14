use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Client;
use super::actions::{ReadOutcome, classify_read};
use super::pagination;
use super::response;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchProtectionState {
    pub required_pull_request_reviews: bool,
    pub required_approving_review_count: u32,
    pub dismiss_stale_reviews: bool,
    pub require_code_owner_reviews: bool,
    pub required_status_checks: bool,
    pub strict_status_checks: bool,
    pub enforce_admins: bool,
    pub required_linear_history: bool,
    pub allow_force_pushes: bool,
    pub allow_deletions: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtectedBranchSummary {
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusCheckRequirement {
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserActor {
    pub login: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamActor {
    pub slug: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppActor {
    pub slug: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorSet {
    #[serde(default)]
    pub users: Vec<UserActor>,
    #[serde(default)]
    pub teams: Vec<TeamActor>,
    #[serde(default)]
    pub apps: Vec<AppActor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PullRequestReviewState {
    #[serde(default)]
    pub dismissal_restrictions: ActorSet,
    #[serde(default)]
    pub bypass_pull_request_allowances: ActorSet,
    #[serde(default)]
    pub dismiss_stale_reviews: bool,
    #[serde(default)]
    pub require_code_owner_reviews: bool,
    #[serde(default)]
    pub required_approving_review_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_last_push_approval: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_reviewers: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusChecksState {
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub contexts: Vec<String>,
    #[serde(default)]
    pub checks: Vec<StatusCheckRequirement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnabledFlag {
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetailedBranchProtection {
    #[serde(default)]
    pub required_pull_request_reviews: Option<PullRequestReviewState>,
    #[serde(default)]
    pub required_status_checks: Option<StatusChecksState>,
    #[serde(default)]
    pub enforce_admins: Option<EnabledFlag>,
    #[serde(default)]
    pub restrictions: Option<ActorSet>,
    #[serde(default)]
    pub required_linear_history: Option<EnabledFlag>,
    #[serde(default)]
    pub allow_force_pushes: Option<EnabledFlag>,
    #[serde(default)]
    pub allow_deletions: Option<EnabledFlag>,
    #[serde(default)]
    pub block_creations: Option<EnabledFlag>,
    #[serde(default)]
    pub required_conversation_resolution: Option<EnabledFlag>,
    #[serde(default)]
    pub required_signatures: Option<EnabledFlag>,
    #[serde(default)]
    pub lock_branch: Option<EnabledFlag>,
    #[serde(default)]
    pub allow_fork_syncing: Option<EnabledFlag>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesiredBranchProtection {
    pub required_pull_request_reviews: bool,
    pub required_approving_review_count: u32,
    pub dismiss_stale_reviews: bool,
    pub require_code_owner_reviews: bool,
    pub require_last_push_approval: Option<bool>,
    pub required_status_checks: bool,
    pub strict_status_checks: bool,
    pub status_check_contexts: Vec<String>,
    pub status_checks: Vec<StatusCheckRequirement>,
    pub push_restrictions: ActorSet,
    pub dismissal_restrictions: ActorSet,
    pub pull_request_bypass_allowances: ActorSet,
    pub enforce_admins: bool,
    pub required_linear_history: bool,
    pub allow_force_pushes: bool,
    pub allow_deletions: bool,
    pub block_creations: Option<bool>,
    pub require_conversation_resolution: Option<bool>,
    pub require_signed_commits: Option<bool>,
    pub lock_branch: Option<bool>,
    pub allow_fork_syncing: Option<bool>,
    pub required_reviewers: Option<serde_json::Value>,
}

fn encode_branch(branch: &str) -> String {
    let mut encoded = String::with_capacity(branch.len());
    for byte in branch.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn actor_slugs(actors: &[AppActor]) -> Vec<String> {
    actors.iter().map(|actor| actor.slug.clone()).collect()
}

fn team_slugs(actors: &[TeamActor]) -> Vec<String> {
    actors.iter().map(|actor| actor.slug.clone()).collect()
}

fn user_logins(actors: &[UserActor]) -> Vec<String> {
    actors.iter().map(|actor| actor.login.clone()).collect()
}

impl Client {
    pub async fn list_protected_branches(&self, repo: &str) -> Result<Vec<ProtectedBranchSummary>> {
        pagination::collect_paginated(self, |page| {
            format!(
                "/repos/{}/{repo}/branches?protected=true&per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse protected branches response")
    }

    pub async fn get_branch_protection_detail(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Option<DetailedBranchProtection>> {
        Ok(self
            .read_branch_protection_detail(repo, branch)
            .await?
            .available())
    }

    pub async fn read_branch_protection_detail(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<ReadOutcome<DetailedBranchProtection>> {
        let branch = encode_branch(branch);
        let path = format!("/repos/{}/{repo}/branches/{branch}/protection", self.org);
        classify_read(self.get(&path).await?, "GET", &path, true)
            .await
            .context("Failed to parse branch protection response")
    }

    pub async fn get_branch_protection(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Option<BranchProtectionState>> {
        let Some(body) = self.get_branch_protection_detail(repo, branch).await? else {
            return Ok(None);
        };

        Ok(Some(BranchProtectionState {
            required_pull_request_reviews: body.required_pull_request_reviews.is_some(),
            required_approving_review_count: body
                .required_pull_request_reviews
                .as_ref()
                .map(|reviews| reviews.required_approving_review_count)
                .unwrap_or(0),
            dismiss_stale_reviews: body
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|reviews| reviews.dismiss_stale_reviews),
            require_code_owner_reviews: body
                .required_pull_request_reviews
                .as_ref()
                .is_some_and(|reviews| reviews.require_code_owner_reviews),
            required_status_checks: body.required_status_checks.is_some(),
            strict_status_checks: body
                .required_status_checks
                .as_ref()
                .is_some_and(|checks| checks.strict),
            enforce_admins: body
                .enforce_admins
                .as_ref()
                .is_some_and(|value| value.enabled),
            required_linear_history: body
                .required_linear_history
                .as_ref()
                .is_some_and(|value| value.enabled),
            allow_force_pushes: body
                .allow_force_pushes
                .as_ref()
                .is_some_and(|value| value.enabled),
            allow_deletions: body
                .allow_deletions
                .as_ref()
                .is_some_and(|value| value.enabled),
        }))
    }

    pub async fn update_branch_protection_detailed(
        &self,
        repo: &str,
        branch: &str,
        desired: &DesiredBranchProtection,
    ) -> Result<()> {
        let required_status_checks = if desired.required_status_checks {
            let checks = if desired.status_checks.is_empty() {
                desired
                    .status_check_contexts
                    .iter()
                    .map(|context| json!({ "context": context }))
                    .collect::<Vec<_>>()
            } else {
                desired
                    .status_checks
                    .iter()
                    .map(|check| {
                        json!({
                            "context": check.context,
                            "app_id": check.app_id,
                        })
                    })
                    .collect::<Vec<_>>()
            };
            json!({
                "strict": desired.strict_status_checks,
                "contexts": desired.status_check_contexts,
                "checks": checks,
            })
        } else {
            serde_json::Value::Null
        };

        let required_pull_request_reviews = if desired.required_pull_request_reviews {
            let mut body = json!({
                "dismissal_restrictions": {
                    "users": user_logins(&desired.dismissal_restrictions.users),
                    "teams": team_slugs(&desired.dismissal_restrictions.teams),
                    "apps": actor_slugs(&desired.dismissal_restrictions.apps),
                },
                "dismiss_stale_reviews": desired.dismiss_stale_reviews,
                "require_code_owner_reviews": desired.require_code_owner_reviews,
                "required_approving_review_count": desired.required_approving_review_count,
                "bypass_pull_request_allowances": {
                    "users": user_logins(&desired.pull_request_bypass_allowances.users),
                    "teams": team_slugs(&desired.pull_request_bypass_allowances.teams),
                    "apps": actor_slugs(&desired.pull_request_bypass_allowances.apps),
                },
            });
            if let Some(value) = desired.require_last_push_approval {
                body["require_last_push_approval"] = json!(value);
            }
            if let Some(reviewers) = &desired.required_reviewers {
                body["required_reviewers"] = reviewers.clone();
            }
            body
        } else {
            serde_json::Value::Null
        };

        let restrictions = if desired.push_restrictions.users.is_empty()
            && desired.push_restrictions.teams.is_empty()
            && desired.push_restrictions.apps.is_empty()
        {
            serde_json::Value::Null
        } else {
            json!({
                "users": user_logins(&desired.push_restrictions.users),
                "teams": team_slugs(&desired.push_restrictions.teams),
                "apps": actor_slugs(&desired.push_restrictions.apps),
            })
        };

        let mut body = json!({
            "required_status_checks": required_status_checks,
            "required_pull_request_reviews": required_pull_request_reviews,
            "enforce_admins": desired.enforce_admins,
            "restrictions": restrictions,
            "required_linear_history": desired.required_linear_history,
            "allow_force_pushes": desired.allow_force_pushes,
            "allow_deletions": desired.allow_deletions,
        });

        if let Some(value) = desired.block_creations {
            body["block_creations"] = json!(value);
        }
        if let Some(value) = desired.require_conversation_resolution {
            body["required_conversation_resolution"] = json!(value);
        }
        if let Some(value) = desired.lock_branch {
            body["lock_branch"] = json!(value);
        }
        if let Some(value) = desired.allow_fork_syncing {
            body["allow_fork_syncing"] = json!(value);
        }

        let branch = encode_branch(branch);
        let path = format!("/repos/{}/{repo}/branches/{branch}/protection", self.org);
        response::expect_empty(self.put_json(&path, &body).await?, "PUT", &path).await
    }

    pub async fn update_branch_protection(
        &self,
        repo: &str,
        branch: &str,
        config: &crate::config::manifest::BranchProtectionConfig,
    ) -> Result<()> {
        self.update_branch_protection_detailed(
            repo,
            branch,
            &DesiredBranchProtection {
                required_pull_request_reviews: config.enabled,
                required_approving_review_count: config.required_approvals,
                dismiss_stale_reviews: config.dismiss_stale_reviews,
                require_code_owner_reviews: config.require_code_owner_reviews,
                require_last_push_approval: None,
                required_status_checks: config.require_status_checks,
                strict_status_checks: config.strict_status_checks,
                status_check_contexts: Vec::new(),
                status_checks: Vec::new(),
                push_restrictions: ActorSet::default(),
                dismissal_restrictions: ActorSet::default(),
                pull_request_bypass_allowances: ActorSet::default(),
                enforce_admins: config.enforce_admins,
                required_linear_history: config.required_linear_history,
                allow_force_pushes: config.allow_force_pushes,
                allow_deletions: config.allow_deletions,
                block_creations: None,
                require_conversation_resolution: None,
                require_signed_commits: None,
                lock_branch: None,
                allow_fork_syncing: None,
                required_reviewers: None,
            },
        )
        .await
    }

    pub async fn delete_branch_protection(&self, repo: &str, branch: &str) -> Result<()> {
        let branch = encode_branch(branch);
        let path = format!("/repos/{}/{repo}/branches/{branch}/protection", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn set_required_signatures(
        &self,
        repo: &str,
        branch: &str,
        enabled: bool,
    ) -> Result<()> {
        let branch = encode_branch(branch);
        let path = format!(
            "/repos/{}/{repo}/branches/{branch}/protection/required_signatures",
            self.org
        );
        if enabled {
            response::expect_json::<serde_json::Value>(
                self.post_json(&path, &json!({})).await?,
                "POST",
                &path,
            )
            .await
            .map(|_| ())
        } else {
            response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
        }
    }
}
