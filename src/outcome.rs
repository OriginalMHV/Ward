use std::fmt;
use std::process::ExitCode;

/// A command finished its work and found a problem, as opposed to failing to run.
///
/// Exit codes: `0` success, `1` for an [`Outcome`], `2` for any other error
/// (authentication, network, configuration, invalid arguments).
#[derive(Debug)]
pub enum Outcome {
    /// `ward drift check` found drift or unreadable managed state.
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

/// Fail with [`Outcome::ApplyFailed`] when any repository failed during an apply.
pub fn fail_when_any_failed(failed: &[(String, String)]) -> anyhow::Result<()> {
    if failed.is_empty() {
        return Ok(());
    }
    Err(Outcome::ApplyFailed(format!(
        "{} repositor{} failed during apply; see the errors above",
        failed.len(),
        if failed.len() == 1 { "y" } else { "ies" }
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
    fn other_errors_exit_with_two() {
        let error = anyhow::anyhow!("network down");
        assert_eq!(exit_code(&error), std::process::ExitCode::from(2));
    }
}
