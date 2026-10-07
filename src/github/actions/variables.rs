use super::*;
use crate::github::Client;
use crate::github::encoding::encode_path_segment;
use crate::github::error::ReadContext;
use crate::github::pagination;
use crate::github::response;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct VariablesResponse {
    variables: Vec<ActionsVariable>,
}

impl Client {
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

    /// Paginated read, classified as a [`ReadOutcome`].
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
}

async fn collect_variables(client: &Client, base_path: &str) -> Result<Vec<ActionsVariable>> {
    let separator = if base_path.contains('?') { '&' } else { '?' };
    pagination::collect_paginated_wrapped(
        client,
        30,
        "Actions variables",
        |page| {
            format!(
                "{base_path}{separator}per_page={}&page={}",
                page.per_page, page.number
            )
        },
        |body: VariablesResponse| pagination::WrappedPage {
            items: body.variables,
            total_count: None,
        },
    )
    .await
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
                .read_ctx("Actions variables")?;
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
