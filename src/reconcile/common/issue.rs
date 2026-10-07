//! Reconcile issue vocabulary shared by the actions, environments, and access categories.

use crate::github::actions::WriteOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Observational or non-blocking: the desired configuration could not be
    /// applied as specified, but this does not indicate a failure.
    Warning,
    /// Prevents part of the plan from being applied (unresolved secret,
    /// invalid combination, endpoint not applicable, etc.).
    Blocker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileIssue {
    /// Dotted path identifying what the issue concerns, e.g.
    /// `actions.settings.enabled` or `environments.production.secrets.TOKEN`.
    pub scope: String,
    pub severity: IssueSeverity,
    pub message: String,
}

impl ReconcileIssue {
    pub(crate) fn warning(scope: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            severity: IssueSeverity::Warning,
            message: message.into(),
        }
    }

    pub(crate) fn blocker(scope: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            severity: IssueSeverity::Blocker,
            message: message.into(),
        }
    }
}

pub(crate) fn has_blocker(issues: &[ReconcileIssue]) -> bool {
    issues
        .iter()
        .any(|issue| issue.severity == IssueSeverity::Blocker)
}

pub(crate) fn write_outcome_issue(
    scope: &str,
    outcome: WriteOutcome,
    applied: &mut Vec<String>,
) -> Option<ReconcileIssue> {
    match outcome {
        WriteOutcome::Applied(()) => {
            applied.push(scope.to_owned());
            None
        }
        WriteOutcome::Blocked(reason) => Some(ReconcileIssue::blocker(scope, reason)),
    }
}

pub(crate) fn format_issue(issue: &ReconcileIssue) -> String {
    format!("{}: {}", issue.scope, issue.message)
}
