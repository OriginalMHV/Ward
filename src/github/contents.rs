use std::fmt;
use std::str::FromStr;

use anyhow::{Context, Result};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

use super::Client;
use super::response::{self, ClassifiedResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitObjectType {
    Blob,
    Tree,
    Commit,
}

impl GitObjectType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Commit => "commit",
        }
    }
}

impl fmt::Display for GitObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitEntryMode {
    File,
    Executable,
    Symlink,
    Submodule,
    Tree,
}

impl GitEntryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "100644",
            Self::Executable => "100755",
            Self::Symlink => "120000",
            Self::Submodule => "160000",
            Self::Tree => "040000",
        }
    }

    pub fn object_type(self) -> GitObjectType {
        match self {
            Self::File | Self::Executable | Self::Symlink => GitObjectType::Blob,
            Self::Submodule => GitObjectType::Commit,
            Self::Tree => GitObjectType::Tree,
        }
    }

    pub fn supports_blob_write(self) -> bool {
        matches!(self, Self::File | Self::Executable)
    }
}

impl fmt::Display for GitEntryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GitEntryMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "100644" => Ok(Self::File),
            "100755" => Ok(Self::Executable),
            "120000" => Ok(Self::Symlink),
            "160000" => Ok(Self::Submodule),
            "040000" | "40000" => Ok(Self::Tree),
            other => anyhow::bail!(
                "Unsupported Git mode {other}. Supported modes: 100644, 100755, 120000, 160000, 040000"
            ),
        }
    }
}

impl Serialize for GitEntryMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GitEntryMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Deserialize)]
pub struct FileContent {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub content: Option<String>,
    pub encoding: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GitTreeEntry {
    pub path: String,
    pub mode: Option<GitEntryMode>,
    pub raw_mode: String,
    pub object_type: GitObjectType,
    pub sha: String,
    pub size: Option<u64>,
}

impl GitTreeEntry {
    pub fn mode_display(&self) -> &str {
        &self.raw_mode
    }
}

#[derive(Debug, Clone)]
pub struct GitTreeListing {
    pub sha: String,
    pub truncated: bool,
    pub entries: Vec<GitTreeEntry>,
}

#[derive(Debug, Deserialize)]
struct GitTreeResponse {
    sha: String,
    tree: Vec<RawGitTreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct RawGitTreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    object_type: GitObjectType,
    sha: String,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GitBlobResponse {
    content: String,
    encoding: String,
}

#[derive(Debug, Deserialize)]
struct RepositoryInfo {
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct RefResponse {
    object: RefObject,
}

#[derive(Debug, Deserialize)]
struct RefObject {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct CommitResponse {
    tree: CommitTree,
}

#[derive(Debug, Deserialize)]
struct CommitTree {
    sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitTreeReadStatus {
    Available,
    EmptyRepository,
    PermissionDenied,
    NotFound,
}

#[derive(Debug, Clone)]
pub struct GitTreeReadResult {
    pub status: GitTreeReadStatus,
    pub listing: Option<GitTreeListing>,
    pub detail: Option<String>,
}

pub(crate) fn validate_relative_git_path(path: &str) -> Result<()> {
    if path.is_empty() {
        anyhow::bail!("Git path cannot be empty");
    }
    if path.starts_with('/') {
        anyhow::bail!("Git path must be relative: {path}");
    }
    if path.ends_with('/') {
        anyhow::bail!("Git path must point to a file entry, not a directory: {path}");
    }
    if path.contains('\\') {
        anyhow::bail!("Git path must use '/' separators: {path}");
    }
    if path.as_bytes().contains(&0) {
        anyhow::bail!("Git path contains a NUL byte: {path}");
    }

    for segment in path.split('/') {
        if segment.is_empty() {
            anyhow::bail!("Git path contains an empty segment: {path}");
        }
        if matches!(segment, "." | "..") {
            anyhow::bail!("Git path contains an unsafe segment '{segment}': {path}");
        }
    }

    Ok(())
}

pub(crate) fn validate_git_ref_name(value: &str) -> Result<()> {
    if value.is_empty() {
        anyhow::bail!("Git ref name cannot be empty");
    }
    if value.starts_with('/') || value.ends_with('/') {
        anyhow::bail!("Git ref name cannot start or end with '/': {value}");
    }
    if value.contains('\\') {
        anyhow::bail!("Git ref name must use '/' separators: {value}");
    }
    if value.as_bytes().contains(&0) {
        anyhow::bail!("Git ref name contains a NUL byte: {value}");
    }

    for segment in value.split('/') {
        if segment.is_empty() {
            anyhow::bail!("Git ref name contains an empty segment: {value}");
        }
        if matches!(segment, "." | "..") {
            anyhow::bail!("Git ref name contains an unsafe segment '{segment}': {value}");
        }
    }

    Ok(())
}

fn split_relative_git_path(path: &str) -> Result<Vec<String>> {
    validate_relative_git_path(path)?;
    Ok(path.split('/').map(ToOwned::to_owned).collect())
}

pub(crate) fn split_git_ref_name(value: &str) -> Result<Vec<String>> {
    validate_git_ref_name(value)?;
    Ok(value.split('/').map(ToOwned::to_owned).collect())
}

pub(crate) fn build_repo_api_url(
    org: &str,
    repo: &str,
    extra_segments: &[String],
    query: &[(&str, &str)],
) -> Result<String> {
    let mut url = Url::parse("https://api.github.invalid")
        .context("Failed to initialize GitHub API URL builder")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Failed to encode GitHub API path"))?;
        segments.push("repos");
        segments.push(org);
        segments.push(repo);
        for segment in extra_segments {
            segments.push(segment);
        }
    }
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }

    Ok(match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    })
}

fn decode_base64_payload(encoded: &str) -> Result<Vec<u8>> {
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &cleaned)
        .context("Failed to decode base64 content")
}

impl Client {
    /// Get a file's content from a repo. Returns None if the file doesn't exist.
    pub async fn get_file(
        &self,
        repo: &str,
        path: &str,
        branch: Option<&str>,
    ) -> Result<Option<FileContent>> {
        let mut extra_segments = vec!["contents".to_owned()];
        extra_segments.extend(split_relative_git_path(path)?);
        let mut query = Vec::new();
        if let Some(branch) = branch {
            query.push(("ref", branch));
        }
        let url = build_repo_api_url(&self.org, repo, &extra_segments, &query)?;

        response::optional_json(self.get(&url).await?, "GET", &url)
            .await
            .context("Failed to parse file content response")
    }

    /// Decode base64-encoded file content from the Contents API into raw bytes.
    pub fn decode_content_bytes(content: &FileContent) -> Result<Vec<u8>> {
        let raw = content.content.as_deref().unwrap_or("");
        if raw.is_empty() {
            return Ok(Vec::new());
        }

        match content.encoding.as_deref() {
            Some("base64") | None => decode_base64_payload(raw),
            Some(other) => anyhow::bail!(
                "Unsupported Contents API encoding {other} for {}",
                content.path
            ),
        }
    }

    /// Decode base64-encoded file content from the Contents API.
    pub fn decode_content(content: &FileContent) -> Result<String> {
        String::from_utf8(Self::decode_content_bytes(content)?)
            .context("File content is not valid UTF-8")
    }

    /// Inspect the repository tree recursively using the Git Trees API.
    pub async fn read_git_tree_recursive(
        &self,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<GitTreeReadResult> {
        let branch = match self.resolve_branch_status(repo, branch).await? {
            ResolveBranchStatus::Resolved(branch) => branch,
            ResolveBranchStatus::Empty(detail) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::EmptyRepository,
                    listing: None,
                    detail: Some(detail),
                });
            }
            ResolveBranchStatus::PermissionDenied(detail) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::PermissionDenied,
                    listing: None,
                    detail: Some(detail),
                });
            }
            ResolveBranchStatus::NotFound(detail) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::NotFound,
                    listing: None,
                    detail: Some(detail),
                });
            }
        };

        let mut ref_segments = vec!["git".to_owned(), "ref".to_owned(), "heads".to_owned()];
        ref_segments.extend(split_git_ref_name(&branch)?);
        let ref_url = build_repo_api_url(&self.org, repo, &ref_segments, &[])?;

        let ref_info = match response::classify_json::<RefResponse>(
            self.get(&ref_url).await?,
            "GET",
            &ref_url,
        )
        .await
        .context("Failed to classify ref response")?
        {
            ClassifiedResponse::Success(value) => value,
            ClassifiedResponse::NotFound(error) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::EmptyRepository,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::Forbidden(error) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::PermissionDenied,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::Other(error) if error.status() == Some(StatusCode::CONFLICT) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::EmptyRepository,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::NoContent
            | ClassifiedResponse::Unprocessable(_)
            | ClassifiedResponse::Other(_) => {
                return Err(anyhow::anyhow!(
                    "Failed to resolve ref for {repo}: GET {ref_url}"
                ))
                .context("Unexpected response while reading Git ref");
            }
        };

        let commit_url = build_repo_api_url(
            &self.org,
            repo,
            &[
                "git".to_owned(),
                "commits".to_owned(),
                ref_info.object.sha.clone(),
            ],
            &[],
        )?;
        let commit = match response::classify_json::<CommitResponse>(
            self.get(&commit_url).await?,
            "GET",
            &commit_url,
        )
        .await
        .context("Failed to classify commit response")?
        {
            ClassifiedResponse::Success(value) => value,
            ClassifiedResponse::NotFound(error) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::EmptyRepository,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::Forbidden(error) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::PermissionDenied,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::Other(error) if error.status() == Some(StatusCode::CONFLICT) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::EmptyRepository,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::NoContent
            | ClassifiedResponse::Unprocessable(_)
            | ClassifiedResponse::Other(_) => {
                return Err(anyhow::anyhow!("Failed to resolve commit tree for {repo}"))
                    .context("Unexpected response while reading Git commit");
            }
        };

        let tree_url = build_repo_api_url(
            &self.org,
            repo,
            &[
                "git".to_owned(),
                "trees".to_owned(),
                commit.tree.sha.clone(),
            ],
            &[("recursive", "1")],
        )?;
        let tree = match response::classify_json::<GitTreeResponse>(
            self.get(&tree_url).await?,
            "GET",
            &tree_url,
        )
        .await
        .context("Failed to classify recursive tree response")?
        {
            ClassifiedResponse::Success(value) => value,
            ClassifiedResponse::NotFound(error) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::EmptyRepository,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::Forbidden(error) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::PermissionDenied,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::Other(error) if error.status() == Some(StatusCode::CONFLICT) => {
                return Ok(GitTreeReadResult {
                    status: GitTreeReadStatus::EmptyRepository,
                    listing: None,
                    detail: Some(error.to_string()),
                });
            }
            ClassifiedResponse::NoContent
            | ClassifiedResponse::Unprocessable(_)
            | ClassifiedResponse::Other(_) => {
                return Err(anyhow::anyhow!("Failed to read recursive tree for {repo}"))
                    .context("Unexpected response while reading Git tree");
            }
        };

        let mut entries = tree
            .tree
            .into_iter()
            .map(|entry| GitTreeEntry {
                path: entry.path,
                mode: entry.mode.parse::<GitEntryMode>().ok(),
                raw_mode: entry.mode,
                object_type: entry.object_type,
                sha: entry.sha,
                size: entry.size,
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.path.cmp(&right.path));

        Ok(GitTreeReadResult {
            status: GitTreeReadStatus::Available,
            listing: Some(GitTreeListing {
                sha: tree.sha,
                truncated: tree.truncated,
                entries,
            }),
            detail: None,
        })
    }

    /// Resolve the repository tree recursively using the Git Trees API.
    /// Returns `None` if the repository or ref doesn't exist.
    pub async fn list_git_tree_recursive(
        &self,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<Option<GitTreeListing>> {
        let tree = self.read_git_tree_recursive(repo, branch).await?;
        match tree.status {
            GitTreeReadStatus::Available => Ok(tree.listing),
            GitTreeReadStatus::EmptyRepository | GitTreeReadStatus::NotFound => Ok(None),
            GitTreeReadStatus::PermissionDenied => {
                anyhow::bail!(
                    "{}",
                    tree.detail.unwrap_or_else(|| {
                        format!("Permission denied while reading repository tree for {repo}")
                    })
                )
            }
        }
    }

    /// Retrieve raw blob bytes using the Git Blobs API.
    pub async fn get_blob_bytes(&self, repo: &str, sha: &str) -> Result<Vec<u8>> {
        if sha.is_empty() {
            anyhow::bail!("Blob SHA cannot be empty");
        }

        let url = build_repo_api_url(
            &self.org,
            repo,
            &["git".to_owned(), "blobs".to_owned(), sha.to_owned()],
            &[],
        )?;
        let blob = response::expect_json::<GitBlobResponse>(self.get(&url).await?, "GET", &url)
            .await
            .context("Failed to parse blob response")?;

        if blob.encoding != "base64" {
            anyhow::bail!("Unsupported Git blob encoding {} for {sha}", blob.encoding);
        }

        decode_base64_payload(&blob.content)
    }

    async fn resolve_branch_status(
        &self,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<ResolveBranchStatus> {
        if let Some(branch) = branch {
            validate_git_ref_name(branch)?;
            return Ok(ResolveBranchStatus::Resolved(branch.to_owned()));
        }

        let url = build_repo_api_url(&self.org, repo, &[], &[])?;
        match response::classify_json::<RepositoryInfo>(self.get(&url).await?, "GET", &url)
            .await
            .context("Failed to classify repository response")?
        {
            ClassifiedResponse::Success(repository) => {
                validate_git_ref_name(&repository.default_branch)?;
                Ok(ResolveBranchStatus::Resolved(repository.default_branch))
            }
            ClassifiedResponse::Forbidden(error) => {
                Ok(ResolveBranchStatus::PermissionDenied(error.to_string()))
            }
            ClassifiedResponse::NotFound(error) => {
                Ok(ResolveBranchStatus::NotFound(error.to_string()))
            }
            ClassifiedResponse::Other(error) if error.status() == Some(StatusCode::CONFLICT) => {
                Ok(ResolveBranchStatus::Empty(error.to_string()))
            }
            ClassifiedResponse::NoContent
            | ClassifiedResponse::Unprocessable(_)
            | ClassifiedResponse::Other(_) => Err(anyhow::anyhow!(
                "Unexpected response while resolving default branch for {repo}"
            )),
        }
    }
}

enum ResolveBranchStatus {
    Resolved(String),
    Empty(String),
    PermissionDenied(String),
    NotFound(String),
}

#[cfg(test)]
mod tests {
    use super::{FileContent, GitEntryMode, validate_relative_git_path};
    use crate::github::Client;

    #[test]
    fn decode_content_bytes_ignores_whitespace() {
        let file = FileContent {
            name: "data.bin".to_owned(),
            path: "data.bin".to_owned(),
            sha: "blob-sha".to_owned(),
            size: 4,
            content: Some("AAEC\n/w==".to_owned()),
            encoding: Some("base64".to_owned()),
            kind: Some("file".to_owned()),
        };

        let decoded = Client::decode_content_bytes(&file).unwrap();
        assert_eq!(decoded, vec![0, 1, 2, 255]);
    }

    #[test]
    fn parse_git_entry_mode_accepts_directory_alias() {
        assert_eq!(
            "040000".parse::<GitEntryMode>().unwrap(),
            GitEntryMode::Tree
        );
        assert_eq!("40000".parse::<GitEntryMode>().unwrap(), GitEntryMode::Tree);
    }

    #[test]
    fn validate_relative_git_path_rejects_traversal() {
        let error = validate_relative_git_path("../danger.txt").unwrap_err();
        assert!(error.to_string().contains("unsafe segment"));
    }
}
