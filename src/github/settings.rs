use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use super::Client;
use super::response;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RepoSettings {
    pub has_issues: bool,
    pub has_projects: bool,
    pub has_wiki: bool,
    #[serde(default)]
    pub has_discussions: bool,
    #[serde(default)]
    pub has_pull_requests: bool,
    #[serde(default)]
    pub pull_request_creation_policy: Option<String>,
    pub allow_squash_merge: bool,
    pub allow_merge_commit: bool,
    pub allow_rebase_merge: bool,
    #[serde(default)]
    pub allow_auto_merge: bool,
    pub delete_branch_on_merge: bool,
    #[serde(default)]
    pub allow_update_branch: bool,
    #[serde(default)]
    pub squash_merge_commit_title: Option<String>,
    #[serde(default)]
    pub squash_merge_commit_message: Option<String>,
    #[serde(default)]
    pub merge_commit_title: Option<String>,
    #[serde(default)]
    pub merge_commit_message: Option<String>,
    #[serde(default)]
    pub web_commit_signoff_required: bool,
    #[serde(default)]
    pub use_squash_pr_title_as_default: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RepositoryGeneralSettings {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub allow_forking: bool,
    #[serde(default)]
    pub has_issues: bool,
    #[serde(default)]
    pub has_projects: bool,
    #[serde(default)]
    pub has_wiki: bool,
    #[serde(default)]
    pub has_discussions: bool,
    #[serde(default)]
    pub has_pull_requests: bool,
    #[serde(default)]
    pub pull_request_creation_policy: Option<String>,
    #[serde(default)]
    pub allow_squash_merge: bool,
    #[serde(default)]
    pub allow_merge_commit: bool,
    #[serde(default)]
    pub allow_rebase_merge: bool,
    #[serde(default)]
    pub allow_auto_merge: bool,
    #[serde(default)]
    pub delete_branch_on_merge: bool,
    #[serde(default)]
    pub allow_update_branch: bool,
    #[serde(default)]
    pub use_squash_pr_title_as_default: Option<bool>,
    #[serde(default)]
    pub squash_merge_commit_title: Option<String>,
    #[serde(default)]
    pub squash_merge_commit_message: Option<String>,
    #[serde(default)]
    pub merge_commit_title: Option<String>,
    #[serde(default)]
    pub merge_commit_message: Option<String>,
    #[serde(default)]
    pub web_commit_signoff_required: bool,
}

impl From<RepositoryGeneralSettings> for RepoSettings {
    fn from(value: RepositoryGeneralSettings) -> Self {
        Self {
            has_issues: value.has_issues,
            has_projects: value.has_projects,
            has_wiki: value.has_wiki,
            has_discussions: value.has_discussions,
            has_pull_requests: value.has_pull_requests,
            pull_request_creation_policy: value.pull_request_creation_policy,
            allow_squash_merge: value.allow_squash_merge,
            allow_merge_commit: value.allow_merge_commit,
            allow_rebase_merge: value.allow_rebase_merge,
            allow_auto_merge: value.allow_auto_merge,
            delete_branch_on_merge: value.delete_branch_on_merge,
            allow_update_branch: value.allow_update_branch,
            squash_merge_commit_title: value.squash_merge_commit_title,
            squash_merge_commit_message: value.squash_merge_commit_message,
            merge_commit_title: value.merge_commit_title,
            merge_commit_message: value.merge_commit_message,
            web_commit_signoff_required: value.web_commit_signoff_required,
            use_squash_pr_title_as_default: value.use_squash_pr_title_as_default,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphqlRepositorySettings {
    #[serde(rename = "hasDiscussionsEnabled", default)]
    pub has_discussions_enabled: bool,
    #[serde(rename = "hasSponsorshipsEnabled", default)]
    pub has_sponsorships_enabled: bool,
    #[serde(rename = "issueCreationPolicy", default)]
    pub issue_creation_policy: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphqlRepositoryPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_discussions_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_sponsorships_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_creation_policy: Option<String>,
}

impl GraphqlRepositoryPatch {
    pub fn is_empty(&self) -> bool {
        self.has_discussions_enabled.is_none()
            && self.has_sponsorships_enabled.is_none()
            && self.issue_creation_policy.is_none()
    }

    fn to_input(&self, repository_id: &str) -> Value {
        let mut input = Map::new();
        input.insert("repositoryId".to_owned(), json!(repository_id));
        if let Some(value) = self.has_discussions_enabled {
            input.insert("hasDiscussionsEnabled".to_owned(), json!(value));
        }
        if let Some(value) = self.has_sponsorships_enabled {
            input.insert("hasSponsorshipsEnabled".to_owned(), json!(value));
        }
        if let Some(value) = self.issue_creation_policy.as_deref() {
            input.insert(
                "issueCreationPolicy".to_owned(),
                json!(graphql_enum_value(value)),
            );
        }
        Value::Object(input)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepositoryCustomPropertyValue {
    pub property_name: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomPropertyValueMutation {
    pub property_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ImmutableReleasesState {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub enforced_by_owner: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepositoryLabel {
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedApiResponse<T> {
    Success(T),
    NoContent,
    Forbidden(String),
    NotFound(String),
    Unprocessable(String),
    Conflict(String),
    Other(String),
}

#[derive(Debug, Deserialize, Serialize)]
struct Topics {
    names: Vec<String>,
}

#[derive(Debug, Serialize)]
struct GraphqlRequest<'a, V> {
    query: &'a str,
    variables: &'a V,
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
struct GraphqlEnvelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

impl Client {
    /// Get repository settings.
    pub async fn get_settings(&self, repo: &str) -> Result<RepoSettings> {
        Ok(self.get_repository_general_settings(repo).await?.into())
    }

    /// Get repository settings and metadata from the repository endpoint.
    pub async fn get_repository_general_settings(
        &self,
        repo: &str,
    ) -> Result<RepositoryGeneralSettings> {
        let path = format!("/repos/{}/{repo}", self.org);
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse repository settings response")
    }

    /// Update repository settings.
    pub async fn update_settings(&self, repo: &str, settings: &Value) -> Result<()> {
        let path = format!("/repos/{}/{repo}", self.org);
        response::expect_empty(self.patch_json(&path, settings).await?, "PATCH", &path).await
    }

    pub async fn get_topics(&self, repo: &str) -> Result<Vec<String>> {
        match self.get_topics_classified(repo).await? {
            ClassifiedApiResponse::Success(topics) => Ok(topics),
            ClassifiedApiResponse::NoContent => Ok(Vec::new()),
            ClassifiedApiResponse::Forbidden(message)
            | ClassifiedApiResponse::NotFound(message)
            | ClassifiedApiResponse::Unprocessable(message)
            | ClassifiedApiResponse::Conflict(message)
            | ClassifiedApiResponse::Other(message) => Err(anyhow::Error::msg(message)),
        }
    }

    pub async fn get_topics_classified(
        &self,
        repo: &str,
    ) -> Result<ClassifiedApiResponse<Vec<String>>> {
        let path = format!("/repos/{}/{repo}/topics", self.org);
        Ok(
            match map_classified_json::<Topics>(
                response::classify_json(self.get(&path).await?, "GET", &path).await?,
            )? {
                ClassifiedApiResponse::Success(topics) => {
                    ClassifiedApiResponse::Success(topics.names)
                }
                ClassifiedApiResponse::NoContent => ClassifiedApiResponse::NoContent,
                ClassifiedApiResponse::Forbidden(message) => {
                    ClassifiedApiResponse::Forbidden(message)
                }
                ClassifiedApiResponse::NotFound(message) => {
                    ClassifiedApiResponse::NotFound(message)
                }
                ClassifiedApiResponse::Unprocessable(message) => {
                    ClassifiedApiResponse::Unprocessable(message)
                }
                ClassifiedApiResponse::Conflict(message) => {
                    ClassifiedApiResponse::Conflict(message)
                }
                ClassifiedApiResponse::Other(message) => ClassifiedApiResponse::Other(message),
            },
        )
    }

    pub async fn replace_topics(&self, repo: &str, topics: &[String]) -> Result<()> {
        let body = Topics {
            names: topics.to_vec(),
        };
        let path = format!("/repos/{}/{repo}/topics", self.org);
        response::expect_empty(self.put_json(&path, &body).await?, "PUT", &path).await
    }

    pub async fn get_repository_graphql_settings(
        &self,
        repo: &str,
    ) -> Result<GraphqlRepositorySettings> {
        match self
            .get_repository_graphql_settings_classified(repo)
            .await?
        {
            ClassifiedApiResponse::Success(settings) => Ok(settings),
            ClassifiedApiResponse::NoContent => Err(anyhow::anyhow!(
                "POST /graphql repository settings returned no content"
            )),
            ClassifiedApiResponse::Forbidden(message)
            | ClassifiedApiResponse::NotFound(message)
            | ClassifiedApiResponse::Unprocessable(message)
            | ClassifiedApiResponse::Conflict(message)
            | ClassifiedApiResponse::Other(message) => Err(anyhow::Error::msg(message)),
        }
    }

    pub async fn get_repository_graphql_settings_classified(
        &self,
        repo: &str,
    ) -> Result<ClassifiedApiResponse<GraphqlRepositorySettings>> {
        #[derive(Debug, Deserialize)]
        struct RepositoryResponse {
            repository: Option<GraphqlRepositorySettings>,
        }

        let query = r#"
            query RepositoryGeneralSettings($owner: String!, $name: String!) {
              repository(owner: $owner, name: $name) {
                hasDiscussionsEnabled
                hasSponsorshipsEnabled
                issueCreationPolicy
              }
            }
        "#;
        let path = "/graphql";
        let request = GraphqlRequest {
            query,
            variables: &json!({ "owner": self.org(), "name": repo }),
        };

        match map_classified_json::<GraphqlEnvelope<RepositoryResponse>>(
            response::classify_json(self.post_json(path, &request).await?, "POST", path).await?,
        )? {
            ClassifiedApiResponse::Success(body) => {
                if !body.errors.is_empty() {
                    return Ok(ClassifiedApiResponse::Other(
                        body.errors
                            .into_iter()
                            .map(|error| error.message)
                            .collect::<Vec<_>>()
                            .join("; "),
                    ));
                }
                if let Some(repository) = body.data.and_then(|data| data.repository) {
                    Ok(ClassifiedApiResponse::Success(repository))
                } else {
                    Ok(ClassifiedApiResponse::Other(
                        "GitHub GraphQL did not return repository settings".to_owned(),
                    ))
                }
            }
            ClassifiedApiResponse::NoContent => Ok(ClassifiedApiResponse::NoContent),
            ClassifiedApiResponse::Forbidden(message) => {
                Ok(ClassifiedApiResponse::Forbidden(message))
            }
            ClassifiedApiResponse::NotFound(message) => {
                Ok(ClassifiedApiResponse::NotFound(message))
            }
            ClassifiedApiResponse::Unprocessable(message) => {
                Ok(ClassifiedApiResponse::Unprocessable(message))
            }
            ClassifiedApiResponse::Conflict(message) => {
                Ok(ClassifiedApiResponse::Conflict(message))
            }
            ClassifiedApiResponse::Other(message) => Ok(ClassifiedApiResponse::Other(message)),
        }
    }

    pub async fn update_repository_graphql_settings(
        &self,
        repository_id: &str,
        patch: &GraphqlRepositoryPatch,
    ) -> Result<GraphqlRepositorySettings> {
        #[derive(Deserialize)]
        struct UpdateRepositoryResponse {
            #[serde(rename = "updateRepository")]
            update_repository: UpdateRepositoryPayload,
        }

        #[derive(Deserialize)]
        struct UpdateRepositoryPayload {
            repository: GraphqlRepositorySettings,
        }

        let query = r#"
            mutation UpdateRepositorySettings($input: UpdateRepositoryInput!) {
              updateRepository(input: $input) {
                repository {
                  hasDiscussionsEnabled
                  hasSponsorshipsEnabled
                  issueCreationPolicy
                }
              }
            }
        "#;

        let data: UpdateRepositoryResponse = self
            .graphql(query, &json!({ "input": patch.to_input(repository_id) }))
            .await?;

        Ok(data.update_repository.repository)
    }

    pub async fn get_custom_property_values(
        &self,
        repo: &str,
    ) -> Result<ClassifiedApiResponse<Vec<RepositoryCustomPropertyValue>>> {
        let path = format!("/repos/{}/{repo}/properties/values", self.org);
        let response = self.get(&path).await?;
        map_classified_json(response::classify_json(response, "GET", &path).await?)
    }

    pub async fn update_custom_property_values(
        &self,
        repo: &str,
        values: &[CustomPropertyValueMutation],
    ) -> Result<ClassifiedApiResponse<()>> {
        let properties = values
            .iter()
            .map(|value| {
                json!({
                    "property_name": value.property_name,
                    "value": value.value,
                })
            })
            .collect::<Vec<_>>();

        let path = format!("/repos/{}/{repo}/properties/values", self.org);
        let response = self
            .patch_json(&path, &json!({ "properties": properties }))
            .await?;
        map_classified_empty(response::classify_empty(response, "PATCH", &path).await?)
    }

    pub async fn get_immutable_releases_state(&self, repo: &str) -> Result<ImmutableReleasesState> {
        match self.get_immutable_releases_state_classified(repo).await? {
            ClassifiedApiResponse::Success(state) => Ok(state),
            ClassifiedApiResponse::NoContent => Ok(ImmutableReleasesState {
                enabled: false,
                enforced_by_owner: false,
            }),
            ClassifiedApiResponse::Forbidden(message)
            | ClassifiedApiResponse::NotFound(message)
            | ClassifiedApiResponse::Unprocessable(message)
            | ClassifiedApiResponse::Conflict(message)
            | ClassifiedApiResponse::Other(message) => Err(anyhow::Error::msg(message)),
        }
    }

    pub async fn get_immutable_releases_state_classified(
        &self,
        repo: &str,
    ) -> Result<ClassifiedApiResponse<ImmutableReleasesState>> {
        let path = format!("/repos/{}/{repo}/immutable-releases", self.org);
        Ok(
            match map_classified_json::<ImmutableReleasesState>(
                response::classify_json(self.get(&path).await?, "GET", &path).await?,
            )? {
                ClassifiedApiResponse::Success(state) => ClassifiedApiResponse::Success(state),
                ClassifiedApiResponse::NotFound(_) => {
                    ClassifiedApiResponse::Success(ImmutableReleasesState {
                        enabled: false,
                        enforced_by_owner: false,
                    })
                }
                other => other,
            },
        )
    }

    pub async fn enable_immutable_releases(&self, repo: &str) -> Result<ClassifiedApiResponse<()>> {
        let path = format!("/repos/{}/{repo}/immutable-releases", self.org);
        classify_empty_with_conflict(self.put(&path).await?, "PUT", &path).await
    }

    pub async fn disable_immutable_releases(
        &self,
        repo: &str,
    ) -> Result<ClassifiedApiResponse<()>> {
        let path = format!("/repos/{}/{repo}/immutable-releases", self.org);
        classify_empty_with_conflict(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn list_labels(&self, repo: &str) -> Result<Vec<RepositoryLabel>> {
        match self.list_labels_classified(repo).await? {
            ClassifiedApiResponse::Success(labels) => Ok(labels),
            ClassifiedApiResponse::NoContent => Ok(Vec::new()),
            ClassifiedApiResponse::Forbidden(message)
            | ClassifiedApiResponse::NotFound(message)
            | ClassifiedApiResponse::Unprocessable(message)
            | ClassifiedApiResponse::Conflict(message)
            | ClassifiedApiResponse::Other(message) => Err(anyhow::Error::msg(message)),
        }
    }

    pub async fn list_labels_classified(
        &self,
        repo: &str,
    ) -> Result<ClassifiedApiResponse<Vec<RepositoryLabel>>> {
        let mut page = 1u32;
        let mut labels = Vec::new();

        loop {
            let path = format!("/repos/{}/{repo}/labels?per_page=100&page={page}", self.org);
            match map_classified_json::<Vec<RepositoryLabel>>(
                response::classify_json(self.get(&path).await?, "GET", &path).await?,
            )? {
                ClassifiedApiResponse::Success(mut page_items) => {
                    let item_count = page_items.len();
                    labels.append(&mut page_items);
                    if item_count < 100 {
                        break;
                    }
                }
                ClassifiedApiResponse::NoContent => break,
                ClassifiedApiResponse::Forbidden(message) => {
                    return Ok(ClassifiedApiResponse::Forbidden(message));
                }
                ClassifiedApiResponse::NotFound(message) => {
                    return Ok(ClassifiedApiResponse::NotFound(message));
                }
                ClassifiedApiResponse::Unprocessable(message) => {
                    return Ok(ClassifiedApiResponse::Unprocessable(message));
                }
                ClassifiedApiResponse::Conflict(message) => {
                    return Ok(ClassifiedApiResponse::Conflict(message));
                }
                ClassifiedApiResponse::Other(message) => {
                    return Ok(ClassifiedApiResponse::Other(message));
                }
            }
            page += 1;
        }

        Ok(ClassifiedApiResponse::Success(labels))
    }

    pub async fn create_label(
        &self,
        repo: &str,
        name: &str,
        color: &str,
        description: Option<&str>,
    ) -> Result<RepositoryLabel> {
        let mut body = Map::new();
        body.insert("name".to_owned(), json!(name));
        body.insert("color".to_owned(), json!(color));
        if let Some(description) = description {
            body.insert("description".to_owned(), json!(description));
        }

        let path = format!("/repos/{}/{repo}/labels", self.org);
        response::expect_json(
            self.post_json(&path, &Value::Object(body)).await?,
            "POST",
            &path,
        )
        .await
        .context("Failed to parse created label response")
    }

    pub async fn update_label(
        &self,
        repo: &str,
        name: &str,
        new_name: Option<&str>,
        color: Option<&str>,
        description: Option<&str>,
    ) -> Result<RepositoryLabel> {
        let mut body = Map::new();
        if let Some(new_name) = new_name {
            body.insert("new_name".to_owned(), json!(new_name));
        }
        if let Some(color) = color {
            body.insert("color".to_owned(), json!(color));
        }
        if let Some(description) = description {
            body.insert("description".to_owned(), json!(description));
        }

        let encoded_name = encode_path_segment(name);
        let path = format!("/repos/{}/{repo}/labels/{encoded_name}", self.org);
        response::expect_json(
            self.patch_json(&path, &Value::Object(body)).await?,
            "PATCH",
            &path,
        )
        .await
        .context("Failed to parse updated label response")
    }

    pub async fn delete_label(&self, repo: &str, name: &str) -> Result<()> {
        let encoded_name = encode_path_segment(name);
        let path = format!("/repos/{}/{repo}/labels/{encoded_name}", self.org);
        response::expect_empty(self.delete(&path).await?, "DELETE", &path).await
    }

    pub async fn branch_exists(&self, repo: &str, branch: &str) -> Result<bool> {
        let encoded_branch = encode_path_segment(branch);
        let path = format!("/repos/{}/{repo}/branches/{encoded_branch}", self.org);
        Ok(
            response::optional_json::<Value>(self.get(&path).await?, "GET", &path)
                .await?
                .is_some(),
        )
    }
}

fn graphql_enum_value(value: &str) -> String {
    value.trim().replace(['-', ' '], "_").to_ascii_uppercase()
}

fn map_classified_json<T>(
    response: response::ClassifiedResponse<T>,
) -> Result<ClassifiedApiResponse<T>> {
    Ok(match response {
        response::ClassifiedResponse::Success(value) => ClassifiedApiResponse::Success(value),
        response::ClassifiedResponse::NoContent => ClassifiedApiResponse::NoContent,
        response::ClassifiedResponse::Forbidden(error) => {
            ClassifiedApiResponse::Forbidden(error.to_string())
        }
        response::ClassifiedResponse::NotFound(error) => {
            ClassifiedApiResponse::NotFound(error.to_string())
        }
        response::ClassifiedResponse::Unprocessable(error) => {
            ClassifiedApiResponse::Unprocessable(error.to_string())
        }
        response::ClassifiedResponse::Other(error) => {
            ClassifiedApiResponse::Other(error.to_string())
        }
    })
}

fn map_classified_empty(
    response: response::ClassifiedResponse<()>,
) -> Result<ClassifiedApiResponse<()>> {
    Ok(match response {
        response::ClassifiedResponse::Success(()) => ClassifiedApiResponse::Success(()),
        response::ClassifiedResponse::NoContent => ClassifiedApiResponse::NoContent,
        response::ClassifiedResponse::Forbidden(error) => {
            ClassifiedApiResponse::Forbidden(error.to_string())
        }
        response::ClassifiedResponse::NotFound(error) => {
            ClassifiedApiResponse::NotFound(error.to_string())
        }
        response::ClassifiedResponse::Unprocessable(error) => {
            ClassifiedApiResponse::Unprocessable(error.to_string())
        }
        response::ClassifiedResponse::Other(error) => {
            ClassifiedApiResponse::Other(error.to_string())
        }
    })
}

async fn classify_empty_with_conflict(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<ClassifiedApiResponse<()>> {
    if response.status() == StatusCode::CONFLICT {
        return Ok(ClassifiedApiResponse::Conflict(
            conflict_message(response, method, path).await,
        ));
    }

    map_classified_empty(response::classify_empty(response, method, path).await?)
}

async fn conflict_message(response: reqwest::Response, method: &str, path: &str) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| format!("{method} {path} failed with HTTP {status}"));

    if message.starts_with(method) {
        message
    } else {
        format!("{method} {path} failed with HTTP {status}: {message}")
    }
}

fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        if matches!(
            byte,
            b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'-'
                | b'.'
                | b'_'
                | b'~'
        ) {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}
