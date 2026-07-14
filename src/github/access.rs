//! Repository collaborator and access APIs.

use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Client;
use super::actions::{ReadOutcome, WriteOutcome, classify_read, write_delete, write_empty};
use super::environments::encode_path_segment;
use super::pagination;
use super::response;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollaboratorAffiliation {
    Direct,
    Outside,
    All,
}

impl CollaboratorAffiliation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Outside => "outside",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepositoryCollaborator {
    pub login: String,
    pub permission: String,
    pub outside: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PendingCollaboratorInvitation {
    pub id: u64,
    pub login: String,
    pub permission: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum CollaboratorGrantResult {
    Active,
    PendingInvitation,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepositoryAppInstallation {
    pub id: u64,
    pub app_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CustomRepositoryRole {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NamedRepository {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OrgScopedResourceMetadata {
    pub name: String,
    pub visibility: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CollaboratorApiResponse {
    login: String,
    #[serde(default)]
    permission: Option<String>,
    #[serde(default)]
    role_name: Option<String>,
    #[serde(default)]
    permissions: Option<Value>,
}

impl CollaboratorApiResponse {
    fn into_collaborator(self, outside: bool) -> RepositoryCollaborator {
        RepositoryCollaborator {
            login: self.login,
            permission: effective_permission(self.permission, self.role_name, self.permissions),
            outside,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RepositoryInvitationApiResponse {
    id: u64,
    invitee: Invitee,
    #[serde(default)]
    permission: Option<String>,
    #[serde(default)]
    role_name: Option<String>,
    #[serde(default)]
    permissions: Option<Value>,
}

impl RepositoryInvitationApiResponse {
    fn into_pending(self) -> PendingCollaboratorInvitation {
        PendingCollaboratorInvitation {
            id: self.id,
            login: self.invitee.login,
            permission: effective_permission(self.permission, self.role_name, self.permissions),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Invitee {
    login: String,
}

#[derive(Debug, Deserialize)]
struct CollaboratorInvitationCreated {
    #[serde(rename = "id")]
    _id: u64,
}

#[derive(Debug, Deserialize)]
struct RepositoryIdentity {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct UserInstallationsPage {
    #[serde(default)]
    installations: Vec<UserInstallationApiResponse>,
}

#[derive(Debug, Deserialize)]
struct UserInstallationApiResponse {
    id: u64,
    app_slug: String,
}

#[derive(Debug, Deserialize)]
struct InstallationRepositoriesPage {
    #[serde(default)]
    repositories: Vec<NamedRepository>,
}

#[derive(Debug, Deserialize)]
struct CustomRepositoryRolesPage {
    #[serde(default)]
    custom_roles: Vec<CustomRepositoryRole>,
}

#[derive(Debug, Deserialize)]
struct SelectedRepositoriesPage {
    #[serde(default)]
    repositories: Vec<NamedRepository>,
}

impl Client {
    pub async fn list_repo_collaborators(
        &self,
        repo: &str,
        affiliation: CollaboratorAffiliation,
    ) -> Result<Vec<RepositoryCollaborator>> {
        self.list_repo_collaborators_checked(repo, affiliation)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("collaborator listing unavailable"))
    }

    pub async fn list_repo_collaborators_checked(
        &self,
        repo: &str,
        affiliation: CollaboratorAffiliation,
    ) -> Result<ReadOutcome<Vec<RepositoryCollaborator>>> {
        let outside = affiliation == CollaboratorAffiliation::Outside;
        collect_paginated_checked(self, |page| {
            format!(
                "/repos/{}/{repo}/collaborators?affiliation={}&per_page={}&page={}",
                self.org,
                affiliation.as_str(),
                page.per_page,
                page.number
            )
        })
        .await
        .context("Failed to parse repo collaborators response")
        .map(|outcome| match outcome {
            ReadOutcome::Available(items) => ReadOutcome::Available(
                items
                    .into_iter()
                    .map(|item: CollaboratorApiResponse| item.into_collaborator(outside))
                    .collect(),
            ),
            ReadOutcome::NotApplicable(reason) => ReadOutcome::NotApplicable(reason),
            ReadOutcome::PermissionDenied(reason) => ReadOutcome::PermissionDenied(reason),
            ReadOutcome::Unavailable(reason) => ReadOutcome::Unavailable(reason),
        })
    }

    pub async fn list_repo_invitations(
        &self,
        repo: &str,
    ) -> Result<Vec<PendingCollaboratorInvitation>> {
        self.list_repo_invitations_checked(repo)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("invitation listing unavailable"))
    }

    pub async fn list_repo_invitations_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<PendingCollaboratorInvitation>>> {
        collect_paginated_checked(self, |page| {
            format!(
                "/repos/{}/{repo}/invitations?per_page={}&page={}",
                self.org, page.per_page, page.number
            )
        })
        .await
        .context("Failed to parse repo invitations response")
        .map(|outcome| match outcome {
            ReadOutcome::Available(items) => ReadOutcome::Available(
                items
                    .into_iter()
                    .map(|item: RepositoryInvitationApiResponse| item.into_pending())
                    .collect(),
            ),
            ReadOutcome::NotApplicable(reason) => ReadOutcome::NotApplicable(reason),
            ReadOutcome::PermissionDenied(reason) => ReadOutcome::PermissionDenied(reason),
            ReadOutcome::Unavailable(reason) => ReadOutcome::Unavailable(reason),
        })
    }

    pub async fn add_repo_collaborator(
        &self,
        repo: &str,
        login: &str,
        permission: &str,
    ) -> Result<CollaboratorGrantResult> {
        let body = serde_json::json!({ "permission": permission });
        let encoded_login = encode_path_segment(login);
        let path = format!("/repos/{}/{repo}/collaborators/{encoded_login}", self.org);
        let response = self.put_json(&path, &body).await?;

        match response.status() {
            StatusCode::CREATED => {
                let _created: CollaboratorInvitationCreated =
                    response::expect_json(response, "PUT", &path).await?;
                Ok(CollaboratorGrantResult::PendingInvitation)
            }
            _ => {
                response::expect_empty(response, "PUT", &path).await?;
                Ok(CollaboratorGrantResult::Active)
            }
        }
    }

    pub async fn remove_repo_collaborator(&self, repo: &str, login: &str) -> Result<WriteOutcome> {
        let encoded_login = encode_path_segment(login);
        let path = format!("/repos/{}/{repo}/collaborators/{encoded_login}", self.org);
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn cancel_repo_invitation(
        &self,
        repo: &str,
        invitation_id: u64,
    ) -> Result<WriteOutcome> {
        let path = format!("/repos/{}/{repo}/invitations/{invitation_id}", self.org);
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn list_repo_app_installations_checked(
        &self,
        repo: &str,
    ) -> Result<ReadOutcome<Vec<RepositoryAppInstallation>>> {
        let repo_name = repo.to_owned();
        let installations = match self.list_user_installations_checked().await? {
            ReadOutcome::Available(installations) => installations,
            ReadOutcome::NotApplicable(reason) => return Ok(ReadOutcome::NotApplicable(reason)),
            ReadOutcome::PermissionDenied(reason) => {
                return Ok(ReadOutcome::PermissionDenied(reason));
            }
            ReadOutcome::Unavailable(reason) => return Ok(ReadOutcome::Unavailable(reason)),
        };

        let mut matches = Vec::new();
        for installation in installations {
            let repos = match self
                .list_user_installation_repositories_checked(installation.id)
                .await?
            {
                ReadOutcome::Available(repositories) => repositories,
                ReadOutcome::NotApplicable(reason) => {
                    return Ok(ReadOutcome::NotApplicable(format!(
                        "Repository association lookup for app `{}` was not applicable: {reason}",
                        installation.app_slug
                    )));
                }
                ReadOutcome::PermissionDenied(reason) => {
                    return Ok(ReadOutcome::PermissionDenied(format!(
                        "Repository association lookup for app `{}` was denied: {reason}",
                        installation.app_slug
                    )));
                }
                ReadOutcome::Unavailable(reason) => {
                    return Ok(ReadOutcome::Unavailable(format!(
                        "Repository association lookup for app `{}` was unavailable: {reason}",
                        installation.app_slug
                    )));
                }
            };
            if repos.iter().any(|candidate| candidate.name == repo_name) {
                matches.push(RepositoryAppInstallation {
                    id: installation.id,
                    app_slug: installation.app_slug,
                });
            }
        }

        Ok(ReadOutcome::Available(matches))
    }

    pub async fn list_custom_repository_roles(&self) -> Result<Vec<CustomRepositoryRole>> {
        self.list_custom_repository_roles_checked()
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("custom repository role listing unavailable"))
    }

    pub async fn list_custom_repository_roles_checked(
        &self,
    ) -> Result<ReadOutcome<Vec<CustomRepositoryRole>>> {
        let mut page = pagination::Page::default();
        let mut roles = Vec::new();

        loop {
            let path = format!(
                "/orgs/{}/custom-repository-roles?per_page={}&page={}",
                self.org, page.per_page, page.number
            );
            let payload: CustomRepositoryRolesPage =
                match classify_read(self.get(&path).await?, "GET", &path, false).await? {
                    ReadOutcome::Available(value) => value,
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
            let item_count = payload.custom_roles.len();
            roles.extend(payload.custom_roles);
            if item_count < page.per_page as usize {
                break;
            }
            page = pagination::Page {
                number: page.number + 1,
                ..page
            };
        }

        Ok(ReadOutcome::Available(roles))
    }

    pub async fn get_org_secret_metadata(
        &self,
        name: &str,
    ) -> Result<Option<OrgScopedResourceMetadata>> {
        self.get_org_secret_metadata_checked(name)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("organization secret metadata unavailable"))
    }

    pub async fn get_org_secret_metadata_checked(
        &self,
        name: &str,
    ) -> Result<ReadOutcome<Option<OrgScopedResourceMetadata>>> {
        let encoded_name = encode_path_segment(name);
        let path = format!("/orgs/{}/actions/secrets/{encoded_name}", self.org);
        classify_optional_metadata_checked(self.get(&path).await?, "GET", &path).await
    }

    pub async fn get_org_variable_metadata(
        &self,
        name: &str,
    ) -> Result<Option<OrgScopedResourceMetadata>> {
        self.get_org_variable_metadata_checked(name)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("organization variable metadata unavailable"))
    }

    pub async fn get_org_variable_metadata_checked(
        &self,
        name: &str,
    ) -> Result<ReadOutcome<Option<OrgScopedResourceMetadata>>> {
        let encoded_name = encode_path_segment(name);
        let path = format!("/orgs/{}/actions/variables/{encoded_name}", self.org);
        classify_optional_metadata_checked(self.get(&path).await?, "GET", &path).await
    }

    pub async fn list_org_secret_selected_repositories(
        &self,
        name: &str,
    ) -> Result<Option<Vec<NamedRepository>>> {
        self.list_org_secret_selected_repositories_checked(name)
            .await?
            .available()
            .ok_or_else(|| anyhow::anyhow!("organization secret selected repositories unavailable"))
    }

    pub async fn list_org_secret_selected_repositories_checked(
        &self,
        name: &str,
    ) -> Result<ReadOutcome<Option<Vec<NamedRepository>>>> {
        self.list_selected_repositories_checked(SelectedRepositoryKind::Secret, name)
            .await
    }

    pub async fn list_org_variable_selected_repositories(
        &self,
        name: &str,
    ) -> Result<Option<Vec<NamedRepository>>> {
        self.list_org_variable_selected_repositories_checked(name)
            .await?
            .available()
            .ok_or_else(|| {
                anyhow::anyhow!("organization variable selected repositories unavailable")
            })
    }

    pub async fn list_org_variable_selected_repositories_checked(
        &self,
        name: &str,
    ) -> Result<ReadOutcome<Option<Vec<NamedRepository>>>> {
        self.list_selected_repositories_checked(SelectedRepositoryKind::Variable, name)
            .await
    }

    pub async fn associate_org_secret_with_repo(
        &self,
        name: &str,
        repo: &str,
    ) -> Result<WriteOutcome> {
        self.associate_selected_repository(SelectedRepositoryKind::Secret, name, repo)
            .await
    }

    pub async fn associate_org_variable_with_repo(
        &self,
        name: &str,
        repo: &str,
    ) -> Result<WriteOutcome> {
        self.associate_selected_repository(SelectedRepositoryKind::Variable, name, repo)
            .await
    }

    pub async fn disassociate_org_secret_from_repo(
        &self,
        name: &str,
        repo: &str,
    ) -> Result<WriteOutcome> {
        self.disassociate_selected_repository(SelectedRepositoryKind::Secret, name, repo)
            .await
    }

    pub async fn disassociate_org_variable_from_repo(
        &self,
        name: &str,
        repo: &str,
    ) -> Result<WriteOutcome> {
        self.disassociate_selected_repository(SelectedRepositoryKind::Variable, name, repo)
            .await
    }

    async fn list_user_installations_checked(
        &self,
    ) -> Result<ReadOutcome<Vec<UserInstallationApiResponse>>> {
        let mut page = pagination::Page::default();
        let mut installations = Vec::new();
        loop {
            let path = format!(
                "/user/installations?per_page={}&page={}",
                page.per_page, page.number
            );
            let payload: UserInstallationsPage =
                match classify_read(self.get(&path).await?, "GET", &path, false).await? {
                    ReadOutcome::Available(value) => value,
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
            let count = payload.installations.len();
            installations.extend(payload.installations);
            if count < page.per_page as usize {
                break;
            }
            page = pagination::Page {
                number: page.number + 1,
                ..page
            };
        }
        Ok(ReadOutcome::Available(installations))
    }

    async fn list_user_installation_repositories_checked(
        &self,
        installation_id: u64,
    ) -> Result<ReadOutcome<Vec<NamedRepository>>> {
        let mut page = pagination::Page::default();
        let mut repositories = Vec::new();
        loop {
            let path = format!(
                "/user/installations/{installation_id}/repositories?per_page={}&page={}",
                page.per_page, page.number
            );
            let payload: InstallationRepositoriesPage =
                match classify_read(self.get(&path).await?, "GET", &path, false).await? {
                    ReadOutcome::Available(value) => value,
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
            let count = payload.repositories.len();
            repositories.extend(payload.repositories);
            if count < page.per_page as usize {
                break;
            }
            page = pagination::Page {
                number: page.number + 1,
                ..page
            };
        }
        Ok(ReadOutcome::Available(repositories))
    }

    async fn list_selected_repositories_checked(
        &self,
        kind: SelectedRepositoryKind,
        name: &str,
    ) -> Result<ReadOutcome<Option<Vec<NamedRepository>>>> {
        let mut page = pagination::Page::default();
        let mut repositories = Vec::new();

        loop {
            let path = kind.repositories_path(&self.org, name, page);
            match classify_read(self.get(&path).await?, "GET", &path, false).await? {
                ReadOutcome::Available(payload) => {
                    let payload: SelectedRepositoriesPage = payload;
                    let item_count = payload.repositories.len();
                    repositories.extend(payload.repositories);
                    if item_count < page.per_page as usize {
                        break;
                    }
                    page = pagination::Page {
                        number: page.number + 1,
                        ..page
                    };
                }
                ReadOutcome::NotApplicable(reason) => {
                    return Ok(ReadOutcome::NotApplicable(reason));
                }
                ReadOutcome::PermissionDenied(reason) => {
                    return Ok(ReadOutcome::PermissionDenied(reason));
                }
                ReadOutcome::Unavailable(reason) => return Ok(ReadOutcome::Unavailable(reason)),
            }
        }

        Ok(ReadOutcome::Available(Some(repositories)))
    }

    async fn associate_selected_repository(
        &self,
        kind: SelectedRepositoryKind,
        name: &str,
        repo: &str,
    ) -> Result<WriteOutcome> {
        let repo_id = self.get_repository_id(repo).await?;
        let path = kind.association_path(&self.org, name, repo_id);
        write_empty(self.put(&path).await?, "PUT", &path).await
    }

    async fn disassociate_selected_repository(
        &self,
        kind: SelectedRepositoryKind,
        name: &str,
        repo: &str,
    ) -> Result<WriteOutcome> {
        let repo_id = self.get_repository_id(repo).await?;
        let path = kind.association_path(&self.org, name, repo_id);
        write_delete(self.delete(&path).await?, "DELETE", &path).await
    }

    async fn get_repository_id(&self, repo: &str) -> Result<u64> {
        let path = format!("/repos/{}/{repo}", self.org);
        let repository: RepositoryIdentity =
            response::expect_json(self.get(&path).await?, "GET", &path)
                .await
                .context("Failed to parse repository identity response")?;
        Ok(repository.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectedRepositoryKind {
    Secret,
    Variable,
}

impl SelectedRepositoryKind {
    fn repositories_path(self, org: &str, name: &str, page: pagination::Page) -> String {
        let encoded_name = encode_path_segment(name);
        match self {
            Self::Secret => format!(
                "/orgs/{org}/actions/secrets/{encoded_name}/repositories?per_page={}&page={}",
                page.per_page, page.number
            ),
            Self::Variable => format!(
                "/orgs/{org}/actions/variables/{encoded_name}/repositories?per_page={}&page={}",
                page.per_page, page.number
            ),
        }
    }

    fn association_path(self, org: &str, name: &str, repo_id: u64) -> String {
        let encoded_name = encode_path_segment(name);
        match self {
            Self::Secret => {
                format!("/orgs/{org}/actions/secrets/{encoded_name}/repositories/{repo_id}")
            }
            Self::Variable => {
                format!("/orgs/{org}/actions/variables/{encoded_name}/repositories/{repo_id}")
            }
        }
    }
}

async fn collect_paginated_checked<T, F>(
    client: &Client,
    mut build_path: F,
) -> Result<ReadOutcome<Vec<T>>>
where
    T: DeserializeOwned,
    F: FnMut(pagination::Page) -> String,
{
    let mut page = pagination::Page::default();
    let mut items = Vec::new();

    loop {
        let path = build_path(page);
        let page_items: Vec<T> = match classify_read(client.get(&path).await?, "GET", &path, false)
            .await?
        {
            ReadOutcome::Available(values) => values,
            ReadOutcome::NotApplicable(reason) => return Ok(ReadOutcome::NotApplicable(reason)),
            ReadOutcome::PermissionDenied(reason) => {
                return Ok(ReadOutcome::PermissionDenied(reason));
            }
            ReadOutcome::Unavailable(reason) => return Ok(ReadOutcome::Unavailable(reason)),
        };
        let count = page_items.len();
        items.extend(page_items);
        if count < page.per_page as usize {
            break;
        }
        page = pagination::Page {
            number: page.number + 1,
            ..page
        };
    }

    Ok(ReadOutcome::Available(items))
}

async fn classify_optional_metadata_checked(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<ReadOutcome<Option<OrgScopedResourceMetadata>>> {
    match response::classify_json(response, method, path).await? {
        response::ClassifiedResponse::Success(value) => Ok(ReadOutcome::Available(Some(value))),
        response::ClassifiedResponse::NotFound(_) => Ok(ReadOutcome::Available(None)),
        response::ClassifiedResponse::Forbidden(error) => {
            Ok(ReadOutcome::PermissionDenied(error.to_string()))
        }
        response::ClassifiedResponse::Unprocessable(error) => {
            Ok(ReadOutcome::NotApplicable(error.to_string()))
        }
        response::ClassifiedResponse::Other(error) => {
            Ok(ReadOutcome::Unavailable(error.to_string()))
        }
        response::ClassifiedResponse::NoContent => Ok(ReadOutcome::Available(None)),
    }
}

fn effective_permission(
    permission: Option<String>,
    role_name: Option<String>,
    permissions: Option<Value>,
) -> String {
    role_name
        .or(permission)
        .or_else(|| permissions.as_ref().and_then(permission_from_value))
        .unwrap_or_else(|| "pull".to_owned())
}

fn permission_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(permission) => Some(permission.clone()),
        Value::Object(map) => permission_from_map(map),
        _ => None,
    }
}

fn permission_from_map(map: &serde_json::Map<String, Value>) -> Option<String> {
    for permission in ["admin", "maintain", "push", "triage", "pull"] {
        if map
            .get(permission)
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Some(permission.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{Client, CollaboratorAffiliation, CollaboratorGrantResult, ReadOutcome};

    #[tokio::test]
    async fn paginates_collaborators_and_preserves_custom_roles() {
        let server = MockServer::start().await;
        let first_page = (0..100)
            .map(|index| {
                json!({
                    "login": format!("user-{index:03}"),
                    "permissions": {
                        "pull": true,
                        "triage": true,
                        "push": true,
                        "maintain": false,
                        "admin": false
                    }
                })
            })
            .collect::<Vec<_>>();

        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/collaborators"))
            .and(query_param("affiliation", "direct"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first_page))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/test-org/my-repo/collaborators"))
            .and(query_param("affiliation", "direct"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![json!({
                "login": "role-user",
                "role_name": "Repository custom"
            })]))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let collaborators = client
            .list_repo_collaborators("my-repo", CollaboratorAffiliation::Direct)
            .await
            .unwrap();

        assert_eq!(collaborators.len(), 101);
        assert_eq!(
            collaborators.last().unwrap().permission,
            "Repository custom"
        );
    }

    #[tokio::test]
    async fn collaborator_put_reports_pending_invitation() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/test-org/my-repo/collaborators/octocat"))
            .and(body_partial_json(json!({ "permission": "push" })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 99 })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let result = client
            .add_repo_collaborator("my-repo", "octocat", "push")
            .await
            .unwrap();

        assert_eq!(result, CollaboratorGrantResult::PendingInvitation);
    }

    #[tokio::test]
    async fn app_lookup_uses_documented_user_installation_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/installations"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "installations": [{ "id": 7, "app_slug": "my-app" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user/installations/7/repositories"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "repositories": [{ "id": 1, "name": "my-repo" }]
            })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let outcome = client
            .list_repo_app_installations_checked("my-repo")
            .await
            .unwrap();

        match outcome {
            ReadOutcome::Available(installations) => {
                assert_eq!(installations[0].app_slug, "my-app");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
}
