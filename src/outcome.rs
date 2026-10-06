use std::fmt;
use std::process::ExitCode;

/// A command finished its work and found a problem, as opposed to failing to run.
///
/// Exit codes: `0` success, `1` for an [`Outcome`], `2` for any other error
/// (authentication, network, configuration, invalid arguments).
#[derive(Debug)]
pub enum Outcome {
    /// `ward drift` found drift or unreadable managed state.
    Drift(String),
    /// A diagnostic or verification check failed.
    ChecksFailed(String),
    /// One or more categories failed or were blocked during apply.
    ApplyFailed(String),
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Self::Drift(message) | Self::ChecksFailed(message) | Self::ApplyFailed(message)) =
            self;
        f.write_str(message)
    }
}

impl std::error::Error for Outcome {}

fn count(n: usize, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

/// Join the non-zero parts with commas. `None` when every count is zero.
fn non_zero(parts: &[(usize, &str, &str, &str)]) -> Option<String> {
    let parts: Vec<String> = parts
        .iter()
        .filter(|(n, ..)| *n > 0)
        .map(|(n, singular, plural, verb)| {
            let noun = count(*n, singular, plural);
            if verb.is_empty() {
                noun
            } else {
                format!("{noun} {verb}")
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// The summary line for an apply with failed or blocked categories.
pub fn apply_summary(failed: usize, blocked: usize) -> String {
    let problems = non_zero(&[
        (failed, "category", "categories", "failed"),
        (blocked, "category", "categories", "blocked"),
    ])
    .unwrap_or_else(|| "unknown problems".to_owned());
    format!("Apply finished with problems: {problems}. The reason is shown under each category.")
}

/// The summary line for a drift check that found drift.
pub fn drift_summary(
    actionable: usize,
    deferred: usize,
    failed: usize,
    blocked: usize,
    unreadable: bool,
) -> String {
    let mut findings = non_zero(&[
        (actionable, "change", "changes", "to apply"),
        (deferred, "deferred change", "deferred changes", ""),
        (failed, "category", "categories", "failed"),
        (blocked, "category", "categories", "blocked"),
    ])
    .unwrap_or_default();
    if unreadable {
        if !findings.is_empty() {
            findings.push_str(". ");
        }
        findings.push_str("Some managed state could not be read");
    }
    format!("Drift found: {findings}. See the report above.")
}

/// Fail with [`Outcome::ApplyFailed`] when any repository failed during an apply.
pub fn fail_when_any_failed(failed: &[(String, String)]) -> anyhow::Result<()> {
    if failed.is_empty() {
        return Ok(());
    }
    Err(Outcome::ApplyFailed(format!(
        "Apply finished with problems: {} failed. The reason is shown above.",
        count(failed.len(), "repository", "repositories")
    ))
    .into())
}

/// Map a command error to the process exit code.
pub fn exit_code(error: &anyhow::Error) -> ExitCode {
    if error.downcast_ref::<Outcome>().is_some() {
        ExitCode::from(1)
    } else {
        ExitCode::from(2)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::{Outcome, exit_code};

    #[test]
    fn outcomes_exit_with_one_even_when_wrapped_in_context() {
        let error = anyhow::Error::new(Outcome::Drift("drift".to_owned())).context("wrapped");
        assert_eq!(exit_code(&error), std::process::ExitCode::from(1));
        let error = Err::<(), _>(Outcome::ApplyFailed("failed".to_owned()))
            .context("apply")
            .unwrap_err();
        assert_eq!(exit_code(&error), std::process::ExitCode::from(1));
    }

    #[test]
    fn failed_repositories_make_an_apply_fail_with_exit_one() {
        assert!(super::fail_when_any_failed(&[]).is_ok());
        let failed = vec![("repo".to_owned(), "boom".to_owned())];
        let error = super::fail_when_any_failed(&failed).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Outcome>(),
            Some(Outcome::ApplyFailed(_))
        ));
        assert_eq!(exit_code(&error), std::process::ExitCode::from(1));
    }

    #[test]
    fn apply_summary_omits_zero_counts() {
        use super::apply_summary;
        assert_eq!(
            apply_summary(1, 0),
            "Apply finished with problems: 1 category failed. The reason is shown under each category."
        );
        assert_eq!(
            apply_summary(0, 2),
            "Apply finished with problems: 2 categories blocked. The reason is shown under each category."
        );
        assert!(apply_summary(2, 1).contains("2 categories failed, 1 category blocked"));
    }

    #[test]
    fn drift_summary_covers_the_combinations() {
        use super::drift_summary;
        assert_eq!(
            drift_summary(2, 0, 0, 1, false),
            "Drift found: 2 changes to apply, 1 category blocked. See the report above."
        );
        assert_eq!(
            drift_summary(0, 0, 0, 0, true),
            "Drift found: Some managed state could not be read. See the report above."
        );
        assert_eq!(
            drift_summary(1, 3, 0, 0, true),
            "Drift found: 1 change to apply, 3 deferred changes. Some managed state could not be read. See the report above."
        );
    }

    #[test]
    fn other_errors_exit_with_two() {
        let error = anyhow::anyhow!("network down");
        assert_eq!(exit_code(&error), std::process::ExitCode::from(2));
    }
}
