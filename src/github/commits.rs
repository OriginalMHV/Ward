use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Client;

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
    entry_type: String,
    sha: String,
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

/// A file to include in an atomic commit.
#[derive(Debug, Clone)]
pub struct CommitFile {
    pub path: String,
    pub content: String,
}

impl Client {
    /// Create an atomic multi-file commit using the Git Trees API.
    /// This avoids cloning the repo — everything happens via the API.
    pub async fn create_commit(
        &self,
        repo: &str,
        branch: &str,
        message: &str,
        files: &[CommitFile],
    ) -> Result<String> {
        let ref_path = format!("/repos/{}/{repo}/git/ref/heads/{branch}", self.org);
        let resp = self.get(&ref_path).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get ref heads/{branch} for {repo} (HTTP {status}): {body}");
        }
        let ref_info: RefResponse = resp.json().await.context("Failed to parse ref response")?;
        let base_commit_sha = ref_info.object.sha;

        // Get the tree SHA for the base commit
        let commit_resp = self
            .get(&format!(
                "/repos/{}/{repo}/git/commits/{base_commit_sha}",
                self.org
            ))
            .await?;
        let commit_data: serde_json::Value = commit_resp.json().await?;
        let base_tree_sha = commit_data["tree"]["sha"]
            .as_str()
            .context("Missing tree SHA in commit")?
            .to_owned();

        // Create blobs for each file
        let mut tree_entries = Vec::new();
        for file in files {
            let blob_req = CreateBlobRequest {
                content: file.content.clone(),
                encoding: "utf-8".to_owned(),
            };

            let resp = self
                .post_json(&format!("/repos/{}/{repo}/git/blobs", self.org), &blob_req)
                .await?;

            let blob: CreateBlobResponse = resp.json().await.context("Failed to create blob")?;

            tree_entries.push(TreeEntry {
                path: file.path.clone(),
                mode: "100644".to_owned(),
                entry_type: "blob".to_owned(),
                sha: blob.sha,
            });
        }

        // Create tree
        let tree_req = CreateTreeRequest {
            base_tree: base_tree_sha,
            tree: tree_entries,
        };

        let resp = self
            .post_json(&format!("/repos/{}/{repo}/git/trees", self.org), &tree_req)
            .await?;
        let tree: CreateTreeResponse = resp.json().await.context("Failed to create tree")?;

        // Create commit
        let commit_req = CreateCommitRequest {
            message: message.to_owned(),
            tree: tree.sha,
            parents: vec![base_commit_sha],
        };

        let resp = self
            .post_json(
                &format!("/repos/{}/{repo}/git/commits", self.org),
                &commit_req,
            )
            .await?;
        let commit: CreateCommitResponse = resp.json().await.context("Failed to create commit")?;

        // Update branch ref
        let update_ref = UpdateRefRequest {
            sha: commit.sha.clone(),
            force: false,
        };

        let resp = self
            .patch_json(
                &format!("/repos/{}/{repo}/git/refs/heads/{branch}", self.org),
                &update_ref,
            )
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to update ref for {repo} (HTTP {status}): {body}");
        }

        Ok(commit.sha)
    }

    /// Create a new branch from the default branch.
    pub async fn create_branch(
        &self,
        repo: &str,
        branch_name: &str,
        from_branch: &str,
    ) -> Result<()> {
        // Get the SHA of the source branch
        let resp = self
            .get(&format!(
                "/repos/{}/{repo}/git/ref/heads/{from_branch}",
                self.org
            ))
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get ref for {from_branch} in {repo}: {body}");
        }

        let ref_info: RefResponse = resp.json().await?;

        let body = serde_json::json!({
            "ref": format!("refs/heads/{branch_name}"),
            "sha": ref_info.object.sha
        });

        let resp = self
            .post_json(&format!("/repos/{}/{repo}/git/refs", self.org), &body)
            .await?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 422 {
            // 422 = branch already exists, which is fine (idempotent)
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to create branch {branch_name} in {repo} (HTTP {status}): {body}"
            );
        }
    }
}
