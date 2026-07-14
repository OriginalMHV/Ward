//! Repository GitHub Actions APIs.
//!
//! Endpoints implemented here are verified against the GitHub REST API
//! reference (`X-GitHub-Api-Version: 2022-11-28`) for:
//! `actions/permissions`, `actions/workflows`, `actions/variables`,
//! `actions/secrets`, `actions/oidc`, `dependabot/secrets`, and
//! `codespaces/repository-secrets`.

use anyhow::{Context, Result, anyhow};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::Client;
use super::environments::encode_path_segment;
use super::response::{self, ClassifiedResponse};

/// The outcome of a write (create/update/delete) call to a GitHub endpoint
/// whose failure modes include expected, non-fatal conditions (validation
/// errors, organization-locked settings, or endpoints that do not apply to a
/// given repository). `Blocked` carries a redacted, human-readable reason
/// (GitHub response bodies are never included verbatim; see
/// [`super::response::GitHubApiError`]'s `Display` impl) so callers can
/// report it without treating it as a hard failure of the whole run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome<T = ()> {
    Applied(T),
    Blocked(String),
}

impl<T> WriteOutcome<T> {
    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied(_))
    }
}

/// Classify a write response with no expected body: 2xx/204 is `Applied`,
/// 403/404/422/other is `Blocked`. Transport-level failures still propagate
/// as `Err`.
pub(crate) async fn write_empty(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<WriteOutcome> {
    match response::classify_empty(response, method, path).await? {
        ClassifiedResponse::Success(()) | ClassifiedResponse::NoContent => {
            Ok(WriteOutcome::Applied(()))
        }
        ClassifiedResponse::NotFound(error)
        | ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Ok(WriteOutcome::Blocked(error.to_string())),
    }
}

/// As [`write_empty`], but a 404 is treated as `Applied` since deleting an
/// already-absent resource is an idempotent no-op, not a blocked action.
pub(crate) async fn write_delete(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<WriteOutcome> {
    match response::classify_empty(response, method, path).await? {
        ClassifiedResponse::Success(())
        | ClassifiedResponse::NoContent
        | ClassifiedResponse::NotFound(_) => Ok(WriteOutcome::Applied(())),
        ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Ok(WriteOutcome::Blocked(error.to_string())),
    }
}

/// The outcome of a *read* against an optional or coverage-tracked GitHub
/// endpoint. Import/collection must never abort because one sub-endpoint is
/// permission-restricted, not applicable to a given repository's visibility,
/// or otherwise unavailable: every caller matches on this instead of using
/// `?` directly, so it can always record a `CoverageEntry` and continue with
/// the rest of collection. Transport-level failures (network errors, JSON
/// decode errors) still propagate as `Err`, since those are genuine
/// anomalies rather than expected per-endpoint conditions.
#[derive(Debug, Clone)]
pub enum ReadOutcome<T> {
    /// The endpoint returned a usable value.
    Available(T),
    /// The endpoint does not apply to this repository (e.g. a private-only
    /// or public-only policy surface), observed as a 404 or 422 whose
    /// message indicates non-applicability rather than a real error.
    NotApplicable(String),
    /// The caller's token lacks the permission required to read this
    /// endpoint (403).
    PermissionDenied(String),
    /// The endpoint is unavailable for some other reason (plan restriction,
    /// unexpected status, etc.).
    Unavailable(String),
}

impl<T> ReadOutcome<T> {
    /// Returns the value if available, discarding the reason otherwise.
    pub fn available(self) -> Option<T> {
        match self {
            Self::Available(value) => Some(value),
            Self::NotApplicable(_) | Self::PermissionDenied(_) | Self::Unavailable(_) => None,
        }
    }
}

/// Classify a JSON read response into a [`ReadOutcome`]. `not_found_is_not_applicable`
/// controls whether a 404 is treated as "this repository doesn't support the
/// endpoint" (`NotApplicable`) or as a genuine unavailability (`Unavailable`);
/// a 422 is always treated as `NotApplicable`, matching GitHub's documented
/// and observed behavior of returning 422 for endpoints that are valid in
/// shape but do not apply to the target repository (e.g. fork PR contributor
/// approval on a private repository).
pub(crate) async fn classify_read<T>(
    response: reqwest::Response,
    method: &str,
    path: &str,
    not_found_is_not_applicable: bool,
) -> Result<ReadOutcome<T>>
where
    T: DeserializeOwned,
{
    match response::classify_json(response, method, path).await? {
        ClassifiedResponse::Success(value) => Ok(ReadOutcome::Available(value)),
        ClassifiedResponse::NoContent => Ok(ReadOutcome::Unavailable(format!(
            "{method} {path} returned HTTP 204 No Content when JSON was expected"
        ))),
        ClassifiedResponse::NotFound(error) => Ok(if not_found_is_not_applicable {
            ReadOutcome::NotApplicable(error.to_string())
        } else {
            ReadOutcome::Unavailable(error.to_string())
        }),
        ClassifiedResponse::Forbidden(error) => {
            Ok(ReadOutcome::PermissionDenied(error.to_string()))
        }
        ClassifiedResponse::Unprocessable(error) => {
            Ok(ReadOutcome::NotApplicable(error.to_string()))
        }
        ClassifiedResponse::Other(error) => Ok(ReadOutcome::Unavailable(error.to_string())),
    }
}

/// Classify a write response that returns the created/updated resource as
/// JSON on success.
pub(crate) async fn write_json<T>(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<WriteOutcome<T>>
where
    T: DeserializeOwned,
{
    match response::classify_json(response, method, path).await? {
        ClassifiedResponse::Success(value) => Ok(WriteOutcome::Applied(value)),
        ClassifiedResponse::NoContent => Ok(WriteOutcome::Blocked(format!(
            "{method} {path} returned HTTP 204 No Content when a resource body was expected"
        ))),
        ClassifiedResponse::NotFound(error)
        | ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Ok(WriteOutcome::Blocked(error.to_string())),
    }
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ActionsPermissions {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_actions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha_pinning_required: Option<bool>,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/selected-actions`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct SelectedActionsPolicy {
    #[serde(default)]
    pub github_owned_allowed: bool,
    #[serde(default)]
    pub verified_allowed: bool,
    #[serde(default)]
    pub patterns_allowed: Vec<String>,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/workflow`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorkflowPermissions {
    pub default_workflow_permissions: String,
    pub can_approve_pull_request_reviews: bool,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/artifact-and-log-retention`.
///
/// GitHub exposes a single combined retention setting for both artifacts and
/// logs; there is no separate REST control for the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ArtifactLogRetention {
    pub days: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_allowed_days: Option<u32>,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/cache/retention-limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ActionsCacheRetentionLimit {
    pub max_cache_retention_days: u32,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/cache/storage-limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ActionsCacheStorageLimit {
    pub max_cache_size_gb: u32,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/fork-pr-contributor-approval`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ForkPrContributorApproval {
    pub approval_policy: String,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/fork-pr-workflows-private-repos`.
///
/// This endpoint only applies to private and internal repositories; GitHub
/// returns 404 for public repositories, which callers should treat as
/// "not applicable" rather than an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct PrivateForkPrWorkflows {
    pub run_workflows_from_fork_pull_requests: bool,
    #[serde(default)]
    pub send_write_tokens_to_workflows: bool,
    #[serde(default)]
    pub send_secrets_and_variables: bool,
    #[serde(default)]
    pub require_approval_for_fork_pr_workflows: bool,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/permissions/access`.
///
/// This endpoint only applies to private repositories.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WorkflowAccessLevel {
    pub access_level: String,
}

/// `GET/PUT /repos/{owner}/{repo}/actions/oidc/customization/sub`.
///
/// `sub_claim_prefix` is computed by GitHub and is not a settable body
/// parameter on the PUT endpoint, so it is observed but never written back.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct OidcSubjectClaim {
    #[serde(default)]
    pub use_default: bool,
    #[serde(default)]
    pub include_claim_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_immutable_subject: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_claim_prefix: Option<String>,
}

/// A single workflow, as returned by `GET /repos/{owner}/{repo}/actions/workflows`.
#[derive(Debug, Clone, Deserialize)]
pub struct Workflow {
    pub id: u64,
    pub path: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowsResponse {
    workflows: Vec<Workflow>,
}

/// A self-hosted runner label, as returned inline on a runner.
#[derive(Debug, Clone, Deserialize)]
pub struct SelfHostedRunnerLabel {
    pub name: String,
}

/// A self-hosted runner, as returned by
/// `GET /repos/{owner}/{repo}/actions/runners`. Ward only ever reads this
/// endpoint: it never registers, re-registers, or deletes runners. The
/// numeric `id`/`runner_group_id` fields are intentionally not modeled here
/// since they must never be persisted as manifest diagnostics (source IDs).
#[derive(Debug, Clone, Deserialize)]
pub struct SelfHostedRunner {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub labels: Vec<SelfHostedRunnerLabel>,
}

#[derive(Debug, Deserialize)]
struct RunnersResponse {
    runners: Vec<SelfHostedRunner>,
}

/// A repository or environment Actions variable.
#[derive(Debug, Clone, Deserialize)]
pub struct ActionsVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
struct VariablesResponse {
    variables: Vec<ActionsVariable>,
}

/// Secret metadata as returned by GitHub. The encrypted value is never
/// returned by any GitHub API and must never be requested or logged.
#[derive(Debug, Clone, Deserialize)]
pub struct SecretMetadata {
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SecretsResponse {
    secrets: Vec<SecretMetadata>,
}

/// The public key used to encrypt secret values for a repository or environment.
#[derive(Debug, Clone, Deserialize)]
pub struct SecretPublicKey {
    pub key_id: String,
    pub key: String,
}

impl Client {
    // ---- Actions permissions ----

    /// `GET /repos/{owner}/{repo}/actions/permissions`.
    pub async fn get_actions_permissions(&self, repo: &str) -> Result<ActionsPermissions> {
        let path = format!("/repos/{}/{repo}/actions/permissions", self.org());
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse Actions permissions response")
    }

    /// As [`Client::get_actions_permissions`], but classifies 403/404/422 as a
    /// [`ReadOutcome`] instead of failing, so collection can proceed with a
    /// `CoverageEntry` when the caller's token lacks permission.
    pub async fn get_actions_permissions_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<ActionsPermissions>> {
        let path = format!("/repos/{}/{repo}/actions/permissions", self.org());
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/permissions`.
    pub async fn set_actions_permissions(
        &self,
        repo: &str,
        permissions: &ActionsPermissions,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/permissions", self.org());
        write_empty(self.put_json(&path, permissions).await?, "PUT", &path).await
    }

    /// `GET /repos/{owner}/{repo}/actions/permissions/selected-actions`.
    pub async fn get_selected_actions(&self, repo: &str) -> Result<SelectedActionsPolicy> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/selected-actions",
            self.org()
        );
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse selected-actions response")
    }

    /// As [`Client::get_selected_actions`], classified.
    pub async fn get_selected_actions_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<SelectedActionsPolicy>> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/selected-actions",
            self.org()
        );
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/permissions/selected-actions`.
    pub async fn set_selected_actions(
        &self,
        repo: &str,
        policy: &SelectedActionsPolicy,
    ) -> Result<WriteOutcome> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/selected-actions",
            self.org()
        );
        write_empty(self.put_json(&path, policy).await?, "PUT", &path).await
    }

    /// `GET /repos/{owner}/{repo}/actions/permissions/workflow`.
    pub async fn get_workflow_permissions(&self, repo: &str) -> Result<WorkflowPermissions> {
        let path = format!("/repos/{}/{repo}/actions/permissions/workflow", self.org());
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse workflow permissions response")
    }

    /// As [`Client::get_workflow_permissions`], classified.
    pub async fn get_workflow_permissions_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<WorkflowPermissions>> {
        let path = format!("/repos/{}/{repo}/actions/permissions/workflow", self.org());
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/permissions/workflow`.
    ///
    /// Returns `409` when the setting is locked by the owning organization;
    /// callers should surface this as a blocked action rather than a failure.
    pub async fn set_workflow_permissions(
        &self,
        repo: &str,
        permissions: &WorkflowPermissions,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/permissions/workflow", self.org());
        write_empty(self.put_json(&path, permissions).await?, "PUT", &path).await
    }

    /// `GET /repos/{owner}/{repo}/actions/permissions/artifact-and-log-retention`.
    ///
    /// Returns `Ok(None)` on 404 (not supported for this repository/plan).
    pub async fn get_artifact_log_retention(
        &self,
        repo: &str,
    ) -> Result<Option<ArtifactLogRetention>> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/artifact-and-log-retention",
            self.org()
        );
        response::optional_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse artifact/log retention response")
    }

    /// As [`Client::get_artifact_log_retention`], classified: 404 is
    /// `Unavailable` (not supported for this repository/plan) rather than
    /// `NotApplicable`, since this endpoint is not visibility-scoped.
    pub async fn get_artifact_log_retention_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<ArtifactLogRetention>> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/artifact-and-log-retention",
            self.org()
        );
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/permissions/artifact-and-log-retention`.
    pub async fn set_artifact_log_retention(&self, repo: &str, days: u32) -> Result<WriteOutcome> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/artifact-and-log-retention",
            self.org()
        );
        write_empty(
            self.put_json(&path, &serde_json::json!({ "days": days }))
                .await?,
            "PUT",
            &path,
        )
        .await
    }

    /// `GET /repos/{owner}/{repo}/actions/cache/retention-limit`. Distinct
    /// from `artifact-and-log-retention` above: this is the retention limit
    /// for GitHub Actions dependency caches (`actions/cache`), not
    /// workflow-run artifacts/logs.
    pub async fn get_actions_cache_retention_limit_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<ActionsCacheRetentionLimit>> {
        let path = format!("/repos/{}/{repo}/actions/cache/retention-limit", self.org());
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/cache/retention-limit`.
    pub async fn set_actions_cache_retention_limit(
        &self,
        repo: &str,
        max_cache_retention_days: u32,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/cache/retention-limit", self.org());
        write_empty(
            self.put_json(
                &path,
                &serde_json::json!({ "max_cache_retention_days": max_cache_retention_days }),
            )
            .await?,
            "PUT",
            &path,
        )
        .await
    }

    /// `GET /repos/{owner}/{repo}/actions/cache/storage-limit`. This is a
    /// writable policy limit; the current cache usage
    /// (`GET .../actions/cache/usage`) is separate runtime data that this
    /// category never collects or manages as desired configuration.
    pub async fn get_actions_cache_storage_limit_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<ActionsCacheStorageLimit>> {
        let path = format!("/repos/{}/{repo}/actions/cache/storage-limit", self.org());
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/cache/storage-limit`.
    pub async fn set_actions_cache_storage_limit(
        &self,
        repo: &str,
        max_cache_size_gb: u32,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/cache/storage-limit", self.org());
        write_empty(
            self.put_json(
                &path,
                &serde_json::json!({ "max_cache_size_gb": max_cache_size_gb }),
            )
            .await?,
            "PUT",
            &path,
        )
        .await
    }

    /// `GET /repos/{owner}/{repo}/actions/permissions/fork-pr-contributor-approval`.
    pub async fn get_fork_pr_contributor_approval(
        &self,
        repo: &str,
    ) -> Result<Option<ForkPrContributorApproval>> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/fork-pr-contributor-approval",
            self.org()
        );
        response::optional_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse fork PR contributor approval response")
    }

    /// As [`Client::get_fork_pr_contributor_approval`], classified. Live
    /// private repositories respond `422 Unprocessable Entity` with a message
    /// such as "Fork PR approval is not allowed for private repositories"
    /// (fork PRs into private repos require an explicit collaborator invite,
    /// so contributor-approval policy does not apply); [`classify_read`]
    /// always treats 422 as [`ReadOutcome::NotApplicable`], so this is
    /// handled without any repository-visibility lookup.
    pub async fn get_fork_pr_contributor_approval_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<ForkPrContributorApproval>> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/fork-pr-contributor-approval",
            self.org()
        );
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/permissions/fork-pr-contributor-approval`.
    pub async fn set_fork_pr_contributor_approval(
        &self,
        repo: &str,
        approval_policy: &str,
    ) -> Result<WriteOutcome> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/fork-pr-contributor-approval",
            self.org()
        );
        write_empty(
            self.put_json(
                &path,
                &serde_json::json!({ "approval_policy": approval_policy }),
            )
            .await?,
            "PUT",
            &path,
        )
        .await
    }

    /// `GET /repos/{owner}/{repo}/actions/permissions/fork-pr-workflows-private-repos`.
    ///
    /// Returns `Ok(None)` on 404, which GitHub returns for public repositories
    /// since this policy only applies to private/internal repositories.
    pub async fn get_private_fork_pr_workflows(
        &self,
        repo: &str,
    ) -> Result<Option<PrivateForkPrWorkflows>> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/fork-pr-workflows-private-repos",
            self.org()
        );
        response::optional_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse private-repo fork PR workflow settings response")
    }

    /// As [`Client::get_private_fork_pr_workflows`], classified: 404 is
    /// `NotApplicable` (public repository).
    pub async fn get_private_fork_pr_workflows_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<PrivateForkPrWorkflows>> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/fork-pr-workflows-private-repos",
            self.org()
        );
        classify_read(self.get(&path).await?, "GET", &path, true).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/permissions/fork-pr-workflows-private-repos`.
    pub async fn set_private_fork_pr_workflows(
        &self,
        repo: &str,
        settings: &PrivateForkPrWorkflows,
    ) -> Result<WriteOutcome> {
        let path = format!(
            "/repos/{}/{repo}/actions/permissions/fork-pr-workflows-private-repos",
            self.org()
        );
        write_empty(self.put_json(&path, settings).await?, "PUT", &path).await
    }

    /// `GET /repos/{owner}/{repo}/actions/permissions/access`.
    ///
    /// Returns `Ok(None)` on 404 (applies only to private repositories).
    pub async fn get_workflow_access_level(
        &self,
        repo: &str,
    ) -> Result<Option<WorkflowAccessLevel>> {
        let path = format!("/repos/{}/{repo}/actions/permissions/access", self.org());
        response::optional_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse workflow access level response")
    }

    /// As [`Client::get_workflow_access_level`], classified: 404 is
    /// `NotApplicable` (public repository).
    pub async fn get_workflow_access_level_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<WorkflowAccessLevel>> {
        let path = format!("/repos/{}/{repo}/actions/permissions/access", self.org());
        classify_read(self.get(&path).await?, "GET", &path, true).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/permissions/access`.
    pub async fn set_workflow_access_level(
        &self,
        repo: &str,
        access_level: &str,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/permissions/access", self.org());
        write_empty(
            self.put_json(&path, &serde_json::json!({ "access_level": access_level }))
                .await?,
            "PUT",
            &path,
        )
        .await
    }

    // ---- OIDC subject claim customization ----

    /// `GET /repos/{owner}/{repo}/actions/oidc/customization/sub`.
    pub async fn get_oidc_subject_claim(&self, repo: &str) -> Result<Option<OidcSubjectClaim>> {
        let path = format!(
            "/repos/{}/{repo}/actions/oidc/customization/sub",
            self.org()
        );
        response::optional_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse OIDC subject claim response")
    }

    /// As [`Client::get_oidc_subject_claim`], classified.
    pub async fn get_oidc_subject_claim_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<OidcSubjectClaim>> {
        let path = format!(
            "/repos/{}/{repo}/actions/oidc/customization/sub",
            self.org()
        );
        classify_read(self.get(&path).await?, "GET", &path, false).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/oidc/customization/sub`. Responds `201`.
    pub async fn set_oidc_subject_claim(
        &self,
        repo: &str,
        use_default: bool,
        include_claim_keys: &[String],
    ) -> Result<WriteOutcome> {
        let path = format!(
            "/repos/{}/{repo}/actions/oidc/customization/sub",
            self.org()
        );
        let body = serde_json::json!({
            "use_default": use_default,
            "include_claim_keys": include_claim_keys,
        });
        write_empty(self.put_json(&path, &body).await?, "PUT", &path).await
    }

    // ---- Workflows (enable/disable, keyed by path) ----

    /// `GET /repos/{owner}/{repo}/actions/workflows`, paginated.
    pub async fn list_workflows(&self, repo: &str) -> Result<Vec<Workflow>> {
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/actions/workflows?per_page=100&page={page}",
                self.org()
            );
            let body: WorkflowsResponse =
                response::expect_json(self.get(&path).await?, "GET", &path)
                    .await
                    .context("Failed to parse workflows response")?;
            let count = body.workflows.len();
            items.extend(body.workflows);
            if count < 100 {
                break;
            }
            page += 1;
        }
        Ok(items)
    }

    /// As [`Client::list_workflows`], classified: a 403/404/422 on the first
    /// page is reported as a [`ReadOutcome`] instead of failing the whole
    /// collection. Used for full-repository workflow enumeration (e.g. a
    /// source import snapshot), where every workflow's enabled state must be
    /// captured rather than only the ones named in a desired configuration.
    pub async fn list_workflows_checked(&self, repo: &str) -> Result<ReadOutcome<Vec<Workflow>>> {
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/actions/workflows?per_page=100&page={page}",
                self.org()
            );
            let response = self.get(&path).await?;
            if page == 1 {
                let body: WorkflowsResponse =
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
                let count = body.workflows.len();
                items.extend(body.workflows);
                if count < 100 {
                    break;
                }
            } else {
                let body: WorkflowsResponse = response::expect_json(response, "GET", &path)
                    .await
                    .context("Failed to parse workflows response")?;
                let count = body.workflows.len();
                items.extend(body.workflows);
                if count < 100 {
                    break;
                }
            }
            page += 1;
        }
        Ok(ReadOutcome::Available(items))
    }

    // ---- Self-hosted runners (read-only diagnostic references) ----
    //
    // Ward NEVER registers, re-registers, or deletes self-hosted runners.
    // These endpoints are read-only observations surfaced as manifest
    // references so drift/inventory is visible, never actionable state.

    /// `GET /repos/{owner}/{repo}/actions/runners`, paginated, classified:
    /// this endpoint is documented for repository scope (unlike runner
    /// groups, which are organization-scoped only).
    pub async fn list_repository_runners_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<SelfHostedRunner>>> {
        let mut items = Vec::new();
        let mut page = 1u32;
        loop {
            let path = format!(
                "/repos/{}/{repo}/actions/runners?per_page=100&page={page}",
                self.org()
            );
            let response = self.get(&path).await?;
            if page == 1 {
                let body: RunnersResponse =
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
                let count = body.runners.len();
                items.extend(body.runners);
                if count < 100 {
                    break;
                }
            } else {
                let body: RunnersResponse = response::expect_json(response, "GET", &path)
                    .await
                    .context("Failed to parse self-hosted runners response")?;
                let count = body.runners.len();
                items.extend(body.runners);
                if count < 100 {
                    break;
                }
            }
            page += 1;
        }
        Ok(ReadOutcome::Available(items))
    }

    /// Find a workflow by its repository-relative file path (e.g. `.github/workflows/ci.yml`).
    pub async fn find_workflow_by_path(
        &self,
        repo: &str,
        workflow_path: &str,
    ) -> Result<Option<Workflow>> {
        Ok(self
            .list_workflows(repo)
            .await?
            .into_iter()
            .find(|workflow| workflow.path == workflow_path))
    }

    /// `PUT /repos/{owner}/{repo}/actions/workflows/{workflow_id}/enable`.
    pub async fn enable_workflow(&self, repo: &str, workflow_id: u64) -> Result<WriteOutcome> {
        let path = format!(
            "/repos/{}/{repo}/actions/workflows/{workflow_id}/enable",
            self.org()
        );
        write_empty(self.put(&path).await?, "PUT", &path).await
    }

    /// `PUT /repos/{owner}/{repo}/actions/workflows/{workflow_id}/disable`.
    pub async fn disable_workflow(&self, repo: &str, workflow_id: u64) -> Result<WriteOutcome> {
        let path = format!(
            "/repos/{}/{repo}/actions/workflows/{workflow_id}/disable",
            self.org()
        );
        write_empty(self.put(&path).await?, "PUT", &path).await
    }

    // ---- Actions variables (repository and environment scoped) ----

    /// `GET /repos/{owner}/{repo}/actions/variables`, paginated.
    pub async fn list_actions_variables(&self, repo: &str) -> Result<Vec<ActionsVariable>> {
        collect_variables(
            self,
            &format!("/repos/{}/{repo}/actions/variables", self.org()),
        )
        .await
    }

    /// As [`Client::list_actions_variables`], classified.
    pub async fn list_actions_variables_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<ActionsVariable>>> {
        collect_variables_checked(
            self,
            &format!("/repos/{}/{repo}/actions/variables", self.org()),
        )
        .await
    }

    /// `POST /repos/{owner}/{repo}/actions/variables`.
    pub async fn create_actions_variable(
        &self,
        repo: &str,
        name: &str,
        value: &str,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/variables", self.org());
        write_empty(
            self.post_json(&path, &serde_json::json!({ "name": name, "value": value }))
                .await?,
            "POST",
            &path,
        )
        .await
    }

    /// `PATCH /repos/{owner}/{repo}/actions/variables/{name}`.
    pub async fn update_actions_variable(
        &self,
        repo: &str,
        name: &str,
        value: &str,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/variables/{name}", self.org());
        write_empty(
            self.patch_json(&path, &serde_json::json!({ "name": name, "value": value }))
                .await?,
            "PATCH",
            &path,
        )
        .await
    }

    /// `DELETE /repos/{owner}/{repo}/actions/variables/{name}`.
    pub async fn delete_actions_variable(&self, repo: &str, name: &str) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/variables/{name}", self.org());
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    /// `GET /repos/{owner}/{repo}/environments/{environment_name}/variables`, paginated.
    pub async fn list_environment_variables(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<Vec<ActionsVariable>> {
        let env = encode_path_segment(environment_name);
        collect_variables(
            self,
            &format!("/repos/{}/{repo}/environments/{env}/variables", self.org()),
        )
        .await
    }

    /// As [`Client::list_environment_variables`], classified.
    pub async fn list_environment_variables_checked(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<ReadOutcome<Vec<ActionsVariable>>> {
        let env = encode_path_segment(environment_name);
        collect_variables_checked(
            self,
            &format!("/repos/{}/{repo}/environments/{env}/variables", self.org()),
        )
        .await
    }

    /// `POST /repos/{owner}/{repo}/environments/{environment_name}/variables`.
    pub async fn create_environment_variable(
        &self,
        repo: &str,
        environment_name: &str,
        name: &str,
        value: &str,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!("/repos/{}/{repo}/environments/{env}/variables", self.org());
        write_empty(
            self.post_json(&path, &serde_json::json!({ "name": name, "value": value }))
                .await?,
            "POST",
            &path,
        )
        .await
    }

    /// `PATCH /repos/{owner}/{repo}/environments/{environment_name}/variables/{name}`.
    pub async fn update_environment_variable(
        &self,
        repo: &str,
        environment_name: &str,
        name: &str,
        value: &str,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/variables/{name}",
            self.org()
        );
        write_empty(
            self.patch_json(&path, &serde_json::json!({ "name": name, "value": value }))
                .await?,
            "PATCH",
            &path,
        )
        .await
    }

    /// `DELETE /repos/{owner}/{repo}/environments/{environment_name}/variables/{name}`.
    pub async fn delete_environment_variable(
        &self,
        repo: &str,
        environment_name: &str,
        name: &str,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/variables/{name}",
            self.org()
        );
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    // ---- Actions secrets (repository and environment scoped) ----

    /// `GET /repos/{owner}/{repo}/actions/secrets/public-key`.
    pub async fn get_actions_public_key(&self, repo: &str) -> Result<SecretPublicKey> {
        let path = format!("/repos/{}/{repo}/actions/secrets/public-key", self.org());
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse Actions public key response")
    }

    /// `GET /repos/{owner}/{repo}/actions/secrets`, paginated. Values are never returned.
    pub async fn list_actions_secrets(&self, repo: &str) -> Result<Vec<SecretMetadata>> {
        collect_secrets(
            self,
            &format!("/repos/{}/{repo}/actions/secrets", self.org()),
        )
        .await
    }

    /// As [`Client::list_actions_secrets`], classified.
    pub async fn list_actions_secrets_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<SecretMetadata>>> {
        collect_secrets_checked(
            self,
            &format!("/repos/{}/{repo}/actions/secrets", self.org()),
        )
        .await
    }

    /// `PUT /repos/{owner}/{repo}/actions/secrets/{name}`. `encrypted_value` must
    /// already be sealed-box encrypted with the repository's public key.
    pub async fn put_actions_secret(
        &self,
        repo: &str,
        name: &str,
        encrypted_value: &str,
        key_id: &str,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/secrets/{name}", self.org());
        write_empty(
            self.put_json(
                &path,
                &serde_json::json!({ "encrypted_value": encrypted_value, "key_id": key_id }),
            )
            .await?,
            "PUT",
            &path,
        )
        .await
    }

    /// `DELETE /repos/{owner}/{repo}/actions/secrets/{name}`.
    pub async fn delete_actions_secret(&self, repo: &str, name: &str) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/actions/secrets/{name}", self.org());
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    /// `GET /repos/{owner}/{repo}/environments/{environment_name}/secrets/public-key`.
    pub async fn get_environment_public_key(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<SecretPublicKey> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/secrets/public-key",
            self.org()
        );
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse environment public key response")
    }

    /// `GET /repos/{owner}/{repo}/environments/{environment_name}/secrets`, paginated.
    pub async fn list_environment_secrets(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<Vec<SecretMetadata>> {
        let env = encode_path_segment(environment_name);
        collect_secrets(
            self,
            &format!("/repos/{}/{repo}/environments/{env}/secrets", self.org()),
        )
        .await
    }

    /// As [`Client::list_environment_secrets`], classified.
    pub async fn list_environment_secrets_checked(
        &self,
        repo: &str,
        environment_name: &str,
    ) -> Result<ReadOutcome<Vec<SecretMetadata>>> {
        let env = encode_path_segment(environment_name);
        collect_secrets_checked(
            self,
            &format!("/repos/{}/{repo}/environments/{env}/secrets", self.org()),
        )
        .await
    }

    /// `PUT /repos/{owner}/{repo}/environments/{environment_name}/secrets/{name}`.
    pub async fn put_environment_secret(
        &self,
        repo: &str,
        environment_name: &str,
        name: &str,
        encrypted_value: &str,
        key_id: &str,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/secrets/{name}",
            self.org()
        );
        write_empty(
            self.put_json(
                &path,
                &serde_json::json!({ "encrypted_value": encrypted_value, "key_id": key_id }),
            )
            .await?,
            "PUT",
            &path,
        )
        .await
    }

    /// `DELETE /repos/{owner}/{repo}/environments/{environment_name}/secrets/{name}`.
    pub async fn delete_environment_secret(
        &self,
        repo: &str,
        environment_name: &str,
        name: &str,
    ) -> Result<WriteOutcome> {
        let env = encode_path_segment(environment_name);
        let path = format!(
            "/repos/{}/{repo}/environments/{env}/secrets/{name}",
            self.org()
        );
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    // ---- Visible organization secret/variable references ----

    /// `GET /repos/{owner}/{repo}/actions/organization-secrets`, paginated.
    /// Names only; GitHub scopes this list to secrets already visible to `repo`.
    pub async fn list_visible_organization_secrets(
        &self,
        repo: &str,
    ) -> Result<Vec<SecretMetadata>> {
        collect_secrets(
            self,
            &format!("/repos/{}/{repo}/actions/organization-secrets", self.org()),
        )
        .await
    }

    /// As [`Client::list_visible_organization_secrets`], classified.
    pub async fn list_visible_organization_secrets_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<SecretMetadata>>> {
        collect_secrets_checked(
            self,
            &format!("/repos/{}/{repo}/actions/organization-secrets", self.org()),
        )
        .await
    }

    /// `GET /repos/{owner}/{repo}/actions/organization-variables`, paginated.
    pub async fn list_visible_organization_variables(
        &self,
        repo: &str,
    ) -> Result<Vec<ActionsVariable>> {
        collect_variables(
            self,
            &format!(
                "/repos/{}/{repo}/actions/organization-variables",
                self.org()
            ),
        )
        .await
    }

    /// As [`Client::list_visible_organization_variables`], classified.
    pub async fn list_visible_organization_variables_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<ActionsVariable>>> {
        collect_variables_checked(
            self,
            &format!(
                "/repos/{}/{repo}/actions/organization-variables",
                self.org()
            ),
        )
        .await
    }

    // ---- Dependabot and Codespaces secret metadata (read-only, no manifest field) ----

    /// `GET /repos/{owner}/{repo}/dependabot/secrets`, paginated. Metadata only;
    /// Dependabot secrets are not part of `ActionsCategoryV2` and are reported
    /// through coverage entries rather than managed state.
    pub async fn list_dependabot_secrets(&self, repo: &str) -> Result<Vec<SecretMetadata>> {
        collect_secrets(
            self,
            &format!("/repos/{}/{repo}/dependabot/secrets", self.org()),
        )
        .await
    }

    /// As [`Client::list_dependabot_secrets`], classified: Dependabot may be
    /// disabled for a repository (404) or restricted (403), neither of which
    /// should abort the rest of Actions/Environments collection.
    pub async fn list_dependabot_secrets_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<SecretMetadata>>> {
        collect_secrets_checked(
            self,
            &format!("/repos/{}/{repo}/dependabot/secrets", self.org()),
        )
        .await
    }

    /// `GET /repos/{owner}/{repo}/codespaces/secrets`, paginated. Metadata only,
    /// reported through coverage entries for the same reason as Dependabot secrets.
    pub async fn list_codespaces_secrets(&self, repo: &str) -> Result<Vec<SecretMetadata>> {
        collect_secrets(
            self,
            &format!("/repos/{}/{repo}/codespaces/secrets", self.org()),
        )
        .await
    }

    /// As [`Client::list_codespaces_secrets`], classified: Codespaces may be
    /// disabled for a repository (404) or restricted (403).
    pub async fn list_codespaces_secrets_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<SecretMetadata>>> {
        collect_secrets_checked(
            self,
            &format!("/repos/{}/{repo}/codespaces/secrets", self.org()),
        )
        .await
    }
}

async fn collect_variables(client: &Client, base_path: &str) -> Result<Vec<ActionsVariable>> {
    let mut items = Vec::new();
    let mut page = 1u32;
    let separator = if base_path.contains('?') { '&' } else { '?' };
    loop {
        let path = format!("{base_path}{separator}per_page=30&page={page}");
        let body: VariablesResponse = response::expect_json(client.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse Actions variables response")?;
        let count = body.variables.len();
        items.extend(body.variables);
        if count < 30 {
            break;
        }
        page += 1;
    }
    Ok(items)
}

/// As [`collect_variables`], but classifies the first page's response so a
/// 403/404/422 becomes a [`ReadOutcome`] instead of aborting.
async fn collect_variables_checked(
    client: &Client,
    base_path: &str,
) -> Result<ReadOutcome<Vec<ActionsVariable>>> {
    let mut items = Vec::new();
    let mut page = 1u32;
    let separator = if base_path.contains('?') { '&' } else { '?' };
    loop {
        let path = format!("{base_path}{separator}per_page=30&page={page}");
        let response = client.get(&path).await?;
        if page == 1 {
            let body: VariablesResponse = match classify_read(response, "GET", &path, false).await?
            {
                ReadOutcome::Available(body) => body,
                ReadOutcome::NotApplicable(reason) => {
                    return Ok(ReadOutcome::NotApplicable(reason));
                }
                ReadOutcome::PermissionDenied(reason) => {
                    return Ok(ReadOutcome::PermissionDenied(reason));
                }
                ReadOutcome::Unavailable(reason) => return Ok(ReadOutcome::Unavailable(reason)),
            };
            let count = body.variables.len();
            items.extend(body.variables);
            if count < 30 {
                break;
            }
        } else {
            let body: VariablesResponse = response::expect_json(response, "GET", &path)
                .await
                .context("Failed to parse Actions variables response")?;
            let count = body.variables.len();
            items.extend(body.variables);
            if count < 30 {
                break;
            }
        }
        page += 1;
    }
    Ok(ReadOutcome::Available(items))
}

async fn collect_secrets(client: &Client, base_path: &str) -> Result<Vec<SecretMetadata>> {
    let mut items = Vec::new();
    let mut page = 1u32;
    let separator = if base_path.contains('?') { '&' } else { '?' };
    loop {
        let path = format!("{base_path}{separator}per_page=30&page={page}");
        let body: SecretsResponse = response::expect_json(client.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse secrets response")?;
        let count = body.secrets.len();
        items.extend(body.secrets);
        if count < 30 {
            break;
        }
        page += 1;
    }
    Ok(items)
}

/// As [`collect_secrets`], but classifies the first page's response so a
/// 403/404/422 becomes a [`ReadOutcome`] instead of aborting.
async fn collect_secrets_checked(
    client: &Client,
    base_path: &str,
) -> Result<ReadOutcome<Vec<SecretMetadata>>> {
    let mut items = Vec::new();
    let mut page = 1u32;
    let separator = if base_path.contains('?') { '&' } else { '?' };
    loop {
        let path = format!("{base_path}{separator}per_page=30&page={page}");
        let response = client.get(&path).await?;
        if page == 1 {
            let body: SecretsResponse = match classify_read(response, "GET", &path, false).await? {
                ReadOutcome::Available(body) => body,
                ReadOutcome::NotApplicable(reason) => {
                    return Ok(ReadOutcome::NotApplicable(reason));
                }
                ReadOutcome::PermissionDenied(reason) => {
                    return Ok(ReadOutcome::PermissionDenied(reason));
                }
                ReadOutcome::Unavailable(reason) => return Ok(ReadOutcome::Unavailable(reason)),
            };
            let count = body.secrets.len();
            items.extend(body.secrets);
            if count < 30 {
                break;
            }
        } else {
            let body: SecretsResponse = response::expect_json(response, "GET", &path)
                .await
                .context("Failed to parse secrets response")?;
            let count = body.secrets.len();
            items.extend(body.secrets);
            if count < 30 {
                break;
            }
        }
        page += 1;
    }
    Ok(ReadOutcome::Available(items))
}

/// Encrypt a plaintext secret value for GitHub using LibSodium-compatible
/// sealed-box encryption (X25519 + XSalsa20-Poly1305), matching the format
/// GitHub's Actions/Dependabot/Codespaces secrets endpoints require.
///
/// The plaintext is never logged. `public_key_base64` must be the `key` field
/// from the corresponding "get public key" endpoint.
pub fn seal_secret_value(public_key_base64: &str, plaintext: &str) -> Result<String> {
    let key_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        public_key_base64,
    )
    .context("Public key is not valid base64")?;
    let key_array: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow!("Public key must be exactly 32 bytes"))?;
    let public_key = crypto_box::PublicKey::from_bytes(key_array);
    let sealed = public_key
        .seal(&mut crypto_box::aead::OsRng, plaintext.as_bytes())
        .map_err(|e| anyhow!("Failed to seal secret value: {e}"))?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        sealed,
    ))
}
