use anyhow::{Context, Result};

/// Resolve a GitHub token for API authentication.
///
/// Priority:
/// 1. `GH_TOKEN` environment variable
/// 2. `GITHUB_TOKEN` environment variable
/// 3. `gh auth token` command output
pub fn resolve_token() -> Result<String> {
    if let Ok(token) = std::env::var("GH_TOKEN") {
        tracing::debug!("Using token from GH_TOKEN");
        return Ok(token);
    }

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        tracing::debug!("Using token from GITHUB_TOKEN");
        return Ok(token);
    }

    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context("Failed to run 'gh auth token' - is the GitHub CLI installed?")?;

    if !output.status.success() {
        anyhow::bail!(
            "gh auth token failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let token = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 from gh auth token")?
        .trim()
        .to_owned();

    if token.is_empty() {
        anyhow::bail!("gh auth token returned empty - run 'gh auth login' first");
    }

    tracing::debug!("Using token from gh auth token");
    Ok(token)
}
