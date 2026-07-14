use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Client;
use super::contents::{
    GitEntryMode, GitObjectType, build_repo_api_url, split_git_ref_name, validate_git_ref_name,
    validate_relative_git_path,
};
use super::response::{self, ClassifiedResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitContent {
    Utf8(String),
    Bytes(Vec<u8>),
    Base64(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicCommitFile {
    pub path: String,
    pub mode: GitEntryMode,
    pub content: CommitContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTreeEntry {
    pub path: String,
    pub mode: GitEntryMode,
    pub object_type: GitObjectType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtomicCommitEntry {
    Upsert(AtomicCommitFile),
    Delete(DeleteTreeEntry),
}

#[derive(Debug, Serialize)]
struct CreateBlobRequest {
    content: String,
    encoding: String,
}

#[derive(Debug, Deserialize)]
struct CreateBlobResponse {
    sha: String,
}

#[derive(Debug, Serialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    entry_type: GitObjectType,
    sha: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateTreeRequest {
    base_tree: String,
    tree: Vec<TreeEntry>,
}

#[derive(Debug, Deserialize)]
struct CreateTreeResponse {
    sha: String,
}

#[derive(Debug, Serialize)]
struct CreateCommitRequest {
    message: String,
    tree: String,
    parents: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CreateCommitResponse {
    sha: String,
}

#[derive(Debug, Serialize)]
struct UpdateRefRequest {
    sha: String,
    force: bool,
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

/// A file to include in an atomic commit.
#[derive(Debug, Clone)]
pub struct CommitFile {
    pub path: String,
    pub content: String,
}

impl Client {
    /// Create an atomic multi-file commit using the Git Trees API.
    /// This avoids cloning the repo - everything happens via the API.
    pub async fn create_commit(
        &self,
        repo: &str,
        branch: &str,
        message: &str,
        files: &[CommitFile],
    ) -> Result<String> {
        let entries = files
            .iter()
            .cloned()
            .map(AtomicCommitEntry::from)
            .collect::<Vec<_>>();

        self.create_atomic_commit(repo, branch, message, &entries)
            .await
    }

    /// Create an atomic commit using explicit Git tree entries.
    pub async fn create_atomic_commit(
        &self,
        repo: &str,
        branch: &str,
        message: &str,
        entries: &[AtomicCommitEntry],
    ) -> Result<String> {
        if entries.is_empty() {
            anyhow::bail!("Refusing to create an empty commit");
        }
        validate_git_ref_name(branch)?;

        let base_commit_sha = self.get_ref_sha(repo, branch).await?;
        let base_tree_sha = self.get_commit_tree_sha(repo, &base_commit_sha).await?;

        let mut tree_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry {
                AtomicCommitEntry::Upsert(file) => {
                    validate_relative_git_path(&file.path)?;
                    ensure_blob_write_mode(file.mode, &file.path)?;

                    let blob_sha = self.create_blob(repo, &file.content).await?;
                    tree_entries.push(TreeEntry {
                        path: file.path.clone(),
                        mode: file.mode.to_string(),
                        entry_type: GitObjectType::Blob,
                        sha: Some(blob_sha),
                    });
                }
                AtomicCommitEntry::Delete(file) => {
                    validate_relative_git_path(&file.path)?;
                    ensure_delete_mode(file.mode, file.object_type, &file.path)?;

                    tree_entries.push(TreeEntry {
                        path: file.path.clone(),
                        mode: file.mode.to_string(),
                        entry_type: file.object_type,
                        sha: None,
                    });
                }
            }
        }

        let tree_req = CreateTreeRequest {
            base_tree: base_tree_sha,
            tree: tree_entries,
        };
        let tree_path = build_repo_api_url(
            &self.org,
            repo,
            &["git".to_owned(), "trees".to_owned()],
            &[],
        )?;
        let tree: CreateTreeResponse = response::expect_json(
            self.post_json(&tree_path, &tree_req).await?,
            "POST",
            &tree_path,
        )
        .await
        .context("Failed to create tree")?;

        let commit_req = CreateCommitRequest {
            message: message.to_owned(),
            tree: tree.sha,
            parents: vec![base_commit_sha],
        };
        let create_commit_path = build_repo_api_url(
            &self.org,
            repo,
            &["git".to_owned(), "commits".to_owned()],
            &[],
        )?;
        let commit: CreateCommitResponse = response::expect_json(
            self.post_json(&create_commit_path, &commit_req).await?,
            "POST",
            &create_commit_path,
        )
        .await
        .context("Failed to create commit")?;

        let update_ref = UpdateRefRequest {
            sha: commit.sha.clone(),
            force: false,
        };
        let update_ref_path = self.branch_ref_url(repo, branch)?;
        response::expect_empty(
            self.patch_json(&update_ref_path, &update_ref).await?,
            "PATCH",
            &update_ref_path,
        )
        .await?;

        Ok(commit.sha)
    }

    /// Create a new branch from the default branch.
    pub async fn create_branch(
        &self,
        repo: &str,
        branch_name: &str,
        from_branch: &str,
    ) -> Result<()> {
        validate_git_ref_name(branch_name)?;
        validate_git_ref_name(from_branch)?;
        let ref_path = self.branch_ref_lookup_url(repo, from_branch)?;
        let ref_info: RefResponse =
            response::expect_json(self.get(&ref_path).await?, "GET", &ref_path).await?;

        let body = serde_json::json!({
            "ref": format!("refs/heads/{branch_name}"),
            "sha": ref_info.object.sha
        });

        let path =
            build_repo_api_url(&self.org, repo, &["git".to_owned(), "refs".to_owned()], &[])?;
        match response::classify_empty(self.post_json(&path, &body).await?, "POST", &path).await? {
            ClassifiedResponse::Success(()) | ClassifiedResponse::NoContent => Ok(()),
            ClassifiedResponse::Unprocessable(error) => {
                if self.get_ref_sha(repo, branch_name).await.is_ok() {
                    Ok(())
                } else {
                    Err(anyhow::Error::new(error))
                }
            }
            ClassifiedResponse::NotFound(error)
            | ClassifiedResponse::Forbidden(error)
            | ClassifiedResponse::Other(error) => Err(anyhow::Error::new(error)),
        }
    }

    /// Ensure a non-default branch exists for file changes and return its name.
    pub async fn ensure_dedicated_branch(
        &self,
        repo: &str,
        branch_name: &str,
        from_branch: &str,
    ) -> Result<String> {
        if branch_name == from_branch {
            anyhow::bail!(
                "Refusing to reuse source branch {from_branch} for file mutations; use a dedicated branch"
            );
        }

        self.create_branch(repo, branch_name, from_branch).await?;
        Ok(branch_name.to_owned())
    }

    pub async fn get_branch_head_sha(&self, repo: &str, branch: &str) -> Result<String> {
        self.get_ref_sha(repo, branch).await
    }

    async fn get_ref_sha(&self, repo: &str, branch: &str) -> Result<String> {
        let ref_path = self.branch_ref_lookup_url(repo, branch)?;
        let ref_info: RefResponse =
            response::expect_json(self.get(&ref_path).await?, "GET", &ref_path)
                .await
                .context("Failed to parse ref response")?;
        Ok(ref_info.object.sha)
    }

    async fn get_commit_tree_sha(&self, repo: &str, commit_sha: &str) -> Result<String> {
        let commit_path = build_repo_api_url(
            &self.org,
            repo,
            &[
                "git".to_owned(),
                "commits".to_owned(),
                commit_sha.to_owned(),
            ],
            &[],
        )?;
        let commit: CommitResponse =
            response::expect_json(self.get(&commit_path).await?, "GET", &commit_path)
                .await
                .context("Failed to parse commit response")?;
        Ok(commit.tree.sha)
    }

    async fn create_blob(&self, repo: &str, content: &CommitContent) -> Result<String> {
        let blob_req = create_blob_request(content)?;
        let blob_path = build_repo_api_url(
            &self.org,
            repo,
            &["git".to_owned(), "blobs".to_owned()],
            &[],
        )?;
        let blob: CreateBlobResponse = response::expect_json(
            self.post_json(&blob_path, &blob_req).await?,
            "POST",
            &blob_path,
        )
        .await
        .context("Failed to create blob")?;
        Ok(blob.sha)
    }

    fn branch_ref_lookup_url(&self, repo: &str, branch: &str) -> Result<String> {
        let mut segments = vec!["git".to_owned(), "ref".to_owned(), "heads".to_owned()];
        segments.extend(split_git_ref_name(branch)?);
        build_repo_api_url(&self.org, repo, &segments, &[])
    }

    fn branch_ref_url(&self, repo: &str, branch: &str) -> Result<String> {
        let mut segments = vec!["git".to_owned(), "refs".to_owned(), "heads".to_owned()];
        segments.extend(split_git_ref_name(branch)?);
        build_repo_api_url(&self.org, repo, &segments, &[])
    }
}

impl From<CommitFile> for AtomicCommitEntry {
    fn from(file: CommitFile) -> Self {
        Self::Upsert(AtomicCommitFile {
            path: file.path,
            mode: GitEntryMode::File,
            content: CommitContent::Utf8(file.content),
        })
    }
}

fn ensure_blob_write_mode(mode: GitEntryMode, path: &str) -> Result<()> {
    if mode.supports_blob_write() {
        return Ok(());
    }

    match mode {
        GitEntryMode::Symlink => {
            anyhow::bail!("Refusing to write symlink payload as an ordinary file: {path}")
        }
        GitEntryMode::Submodule => {
            anyhow::bail!("Refusing to write submodule payload as an ordinary file: {path}")
        }
        GitEntryMode::Tree => {
            anyhow::bail!("Refusing to write a directory tree entry as a file: {path}")
        }
        GitEntryMode::File | GitEntryMode::Executable => Ok(()),
    }
}

fn ensure_delete_mode(mode: GitEntryMode, object_type: GitObjectType, path: &str) -> Result<()> {
    if mode.object_type() != object_type {
        anyhow::bail!(
            "Unsupported Git mode/type combination for {path}: mode {mode} requires type {}, got {object_type}",
            mode.object_type()
        );
    }

    if matches!(mode, GitEntryMode::Tree) {
        anyhow::bail!("Deleting directory tree entries is not supported: {path}");
    }

    Ok(())
}

fn create_blob_request(content: &CommitContent) -> Result<CreateBlobRequest> {
    match content {
        CommitContent::Utf8(text) => Ok(CreateBlobRequest {
            content: text.clone(),
            encoding: "utf-8".to_owned(),
        }),
        CommitContent::Bytes(bytes) => Ok(CreateBlobRequest {
            content: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
            encoding: "base64".to_owned(),
        }),
        CommitContent::Base64(encoded) => {
            let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
            let bytes =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &cleaned)
                    .context("Invalid base64 content")?;

            Ok(CreateBlobRequest {
                content: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
                encoding: "base64".to_owned(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommitContent, create_blob_request, ensure_blob_write_mode, ensure_delete_mode};
    use crate::github::contents::{GitEntryMode, GitObjectType};

    #[test]
    fn create_blob_request_normalizes_base64_input() {
        let request = create_blob_request(&CommitContent::Base64("AAEC\n/w==".to_owned())).unwrap();
        assert_eq!(request.encoding, "base64");
        assert_eq!(request.content, "AAEC/w==");
    }

    #[test]
    fn ensure_blob_write_mode_rejects_symlink() {
        let error = ensure_blob_write_mode(GitEntryMode::Symlink, "link").unwrap_err();
        assert!(error.to_string().contains("symlink"));
    }

    #[test]
    fn ensure_delete_mode_rejects_mismatched_type() {
        let error =
            ensure_delete_mode(GitEntryMode::Submodule, GitObjectType::Blob, "module").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unsupported Git mode/type combination")
        );
    }
}
