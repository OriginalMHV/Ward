use super::*;
use crate::github::Client;
use crate::github::encoding::encode_path_segment;
use crate::github::response;
use anyhow::{Context, Result, anyhow};

#[derive(Debug, Deserialize)]
struct SecretsResponse {
    secrets: Vec<SecretMetadata>,
}

impl Client {
    // ---- Actions secrets (repository and environment scoped) ----

    /// `GET /repos/{owner}/{repo}/actions/secrets/public-key`.
    pub async fn get_actions_public_key(&self, repo: &str) -> Result<SecretPublicKey> {
        let path = format!("/repos/{}/{repo}/actions/secrets/public-key", self.org());
        response::expect_json(self.get(&path).await?, "GET", &path)
            .await
            .context("Failed to parse Actions public key response")
    }

    /// Paginated read, classified as a [`ReadOutcome`].
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

    /// Paginated read, classified as a [`ReadOutcome`].
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

    /// Paginated read, classified as a [`ReadOutcome`].
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

    // ---- Dependabot and Codespaces secret metadata (read-only, no manifest field) ----

    /// Paginated read, classified as a [`ReadOutcome`]: Dependabot may be
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

    /// Paginated read, classified as a [`ReadOutcome`]: Codespaces may be
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

/// Classifies the first page's response so a
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
