use super::*;
use crate::github::Client;
use crate::github::pagination;
use crate::github::response;
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct WorkflowsResponse {
    workflows: Vec<Workflow>,
}

#[derive(Debug, Deserialize)]
struct RunnersResponse {
    runners: Vec<SelfHostedRunner>,
}

impl Client {
    // ---- Workflows (enable/disable, keyed by path) ----

    /// `GET /repos/{owner}/{repo}/actions/workflows`, paginated.
    pub async fn list_workflows(&self, repo: &str) -> Result<Vec<Workflow>> {
        pagination::collect_paginated_wrapped(
            self,
            100,
            "Failed to parse workflows response",
            |page| {
                format!(
                    "/repos/{}/{repo}/actions/workflows?per_page={}&page={}",
                    self.org(),
                    page.per_page,
                    page.number
                )
            },
            |body: WorkflowsResponse| pagination::WrappedPage {
                items: body.workflows,
                total_count: None,
            },
        )
        .await
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
}
