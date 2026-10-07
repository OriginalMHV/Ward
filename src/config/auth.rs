use std::io::ErrorKind;

use anyhow::{Result, bail};

/// What running `gh auth token` produced.
struct GhOutput {
    success: bool,
    stdout: Vec<u8>,
}

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
    resolve_token_with(|name| std::env::var(name).ok(), run_gh_auth_token)
}

fn run_gh_auth_token() -> std::io::Result<GhOutput> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()?;
    Ok(GhOutput {
        success: output.status.success(),
        stdout: output.stdout,
    })
}

/// Return a trimmed, non-empty value for the first listed variable that has one.
fn env_token(lookup: &impl Fn(&str) -> Option<String>) -> Option<(&'static str, String)> {
    ["GH_TOKEN", "GITHUB_TOKEN"].into_iter().find_map(|name| {
        let value = lookup(name)?;
        let token = value.trim();
        (!token.is_empty()).then(|| (name, token.to_owned()))
    })
}

/// Names of the token variables that are set but blank.
fn blank_variables(lookup: &impl Fn(&str) -> Option<String>) -> Vec<&'static str> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .filter(|name| lookup(name).is_some_and(|value| value.trim().is_empty()))
        .collect()
}

fn blank_note(blank: &[&str]) -> String {
    match blank {
        [] => String::new(),
        names => format!(" {} is set but empty.", names.join(" and ")),
    }
}

fn resolve_token_with(
    lookup: impl Fn(&str) -> Option<String>,
    run_gh: impl Fn() -> std::io::Result<GhOutput>,
) -> Result<String> {
    if let Some((name, token)) = env_token(&lookup) {
        tracing::debug!("Using token from {name}");
        return Ok(token);
    }

    let blank = blank_note(&blank_variables(&lookup));

    let output = match run_gh() {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => bail!(
            "No GitHub token found.{blank} The GitHub CLI (gh) is not installed. Set GH_TOKEN or GITHUB_TOKEN to a token, or install gh from https://cli.github.com and run 'gh auth login'."
        ),
        Err(error) => bail!(
            "No GitHub token found.{blank} Ward could not run 'gh auth token' ({error}). Set GH_TOKEN or GITHUB_TOKEN to a token, or fix your gh installation and run 'gh auth login'."
        ),
    };

    let token = if output.success {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    } else {
        String::new()
    };

    if token.is_empty() {
        bail!(
            "No GitHub token found.{blank} The GitHub CLI (gh) is installed but not logged in. Run 'gh auth login', or set GH_TOKEN or GITHUB_TOKEN to a token."
        );
    }

    tracing::debug!("Using token from gh auth token");
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::{GhOutput, env_token, resolve_token_with};
    use std::io::ErrorKind;

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

    fn gh_missing() -> std::io::Result<GhOutput> {
        Err(std::io::Error::from(ErrorKind::NotFound))
    }

    fn gh_logged_out() -> GhOutput {
        GhOutput {
            success: false,
            stdout: Vec::new(),
        }
    }

    #[test]
    fn no_token_and_no_gh_names_both_fixes() {
        let error = resolve_token_with(lookup(&[]), gh_missing).unwrap_err();
        assert_eq!(
            error.to_string(),
            "No GitHub token found. The GitHub CLI (gh) is not installed. Set GH_TOKEN or GITHUB_TOKEN to a token, or install gh from https://cli.github.com and run 'gh auth login'."
        );
    }

    #[test]
    fn gh_without_login_says_to_log_in() {
        let error = resolve_token_with(lookup(&[]), || Ok(gh_logged_out())).unwrap_err();
        assert_eq!(
            error.to_string(),
            "No GitHub token found. The GitHub CLI (gh) is installed but not logged in. Run 'gh auth login', or set GH_TOKEN or GITHUB_TOKEN to a token."
        );
    }

    #[test]
    fn an_empty_token_variable_is_named() {
        let error = resolve_token_with(lookup(&[("GH_TOKEN", " ")]), gh_missing).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("No GitHub token found. GH_TOKEN is set but empty. "),
            "{error}"
        );
    }

    #[test]
    fn an_empty_gh_token_counts_as_not_logged_in() {
        let empty = || {
            Ok(GhOutput {
                success: true,
                stdout: b"\n".to_vec(),
            })
        };
        let error = resolve_token_with(lookup(&[]), empty).unwrap_err();
        assert!(error.to_string().contains("not logged in"), "{error}");
    }

    #[test]
    fn a_gh_token_is_trimmed_and_used() {
        let found = || {
            Ok(GhOutput {
                success: true,
                stdout: b"gho_x\n".to_vec(),
            })
        };
        assert_eq!(resolve_token_with(lookup(&[]), found).unwrap(), "gho_x");
    }
}
