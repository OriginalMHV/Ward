use super::*;
use crate::github::Client;
use crate::github::response;
use anyhow::{Context, Result};

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
}
