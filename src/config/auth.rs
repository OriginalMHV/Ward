use anyhow::{Context, Result};

/// Resolve a GitHub token for API authentication.
///
/// Priority:
/// 1. `GH_TOKEN` environment variable
/// 2. `GITHUB_TOKEN` environment variable
/// 3. `gh auth token` command output
#[allow(
    clippy::disallowed_methods,
    reason = "token resolution entry point; resolve_token_with takes the injected lookup"
)]
pub fn resolve_token() -> Result<String> {
    resolve_token_with(|name| std::env::var(name).ok())
}

/// Return a trimmed, non-empty value for the first listed variable that has one.
fn env_token(lookup: &impl Fn(&str) -> Option<String>) -> Option<(&'static str, String)> {
    ["GH_TOKEN", "GITHUB_TOKEN"].into_iter().find_map(|name| {
        let value = lookup(name)?;
        let token = value.trim();
        (!token.is_empty()).then(|| (name, token.to_owned()))
    })
}

fn resolve_token_with(lookup: impl Fn(&str) -> Option<String>) -> Result<String> {
    if let Some((name, token)) = env_token(&lookup) {
        tracing::debug!("Using token from {name}");
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

#[cfg(test)]
mod tests {
    use super::env_token;

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn environment_tokens_are_trimmed() {
        let token = env_token(&lookup(&[("GH_TOKEN", "  abc\n")]));
        assert_eq!(token, Some(("GH_TOKEN", "abc".to_owned())));
    }

    #[test]
    fn empty_or_blank_tokens_fall_through_to_the_next_source() {
        let token = env_token(&lookup(&[("GH_TOKEN", ""), ("GITHUB_TOKEN", " tok ")]));
        assert_eq!(token, Some(("GITHUB_TOKEN", "tok".to_owned())));
        assert_eq!(env_token(&lookup(&[("GH_TOKEN", "  ")])), None);
    }
}
