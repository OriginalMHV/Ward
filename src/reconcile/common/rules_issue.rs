//! Issue type used by the security, rulesets, and branch-protection categories.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileIssueSeverity {
    Warning,
    Blocker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileIssue {
    pub resource: Option<String>,
    pub code: &'static str,
    pub severity: ReconcileIssueSeverity,
    pub message: String,
}

pub(crate) fn warning_issue(
    resource: Option<String>,
    code: &'static str,
    message: String,
) -> ReconcileIssue {
    ReconcileIssue {
        resource,
        code,
        severity: ReconcileIssueSeverity::Warning,
        message,
    }
}

pub(crate) fn blocker_issue(
    resource: Option<String>,
    code: &'static str,
    message: String,
) -> ReconcileIssue {
    ReconcileIssue {
        resource,
        code,
        severity: ReconcileIssueSeverity::Blocker,
        message,
    }
}
