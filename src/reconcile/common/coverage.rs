//! Coverage entry constructors and read-outcome recording shared across categories.

use crate::config::manifest::{CoverageEntry, CoverageOutcome, ManifestCategoryName};
use crate::github::actions;

pub(crate) fn collected_entry(category: ManifestCategoryName, endpoint: &str) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::Collected,
        reason: None,
        required_permission: None,
    }
}

pub(crate) fn permission_denied_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    reason: String,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::PermissionDenied,
        reason: Some(reason),
        required_permission: None,
    }
}

pub(crate) fn not_applicable_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    reason: String,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::NotApplicable,
        reason: Some(reason),
        required_permission: None,
    }
}

/// Record a failed lookup. A lookup the manifest does not need is not applicable,
/// so it does not make the category count as unknown.
pub(crate) fn lookup_failure_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    reason: String,
    requested: bool,
) -> CoverageEntry {
    if requested {
        unavailable_entry(category, endpoint, reason)
    } else {
        CoverageEntry {
            category,
            endpoint: endpoint.to_owned(),
            outcome: CoverageOutcome::NotApplicable,
            reason: Some(format!("not required by the manifest: {reason}")),
            required_permission: None,
        }
    }
}

pub(crate) fn unavailable_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    reason: String,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome: CoverageOutcome::Unavailable,
        reason: Some(reason),
        required_permission: None,
    }
}

/// Record a classified [`actions::ReadOutcome`] into `coverage` when it is
/// not `Available`, returning the value on success. Used throughout
/// collection so an optional endpoint's 403/404/422/other failure becomes a
/// [`CoverageEntry`] instead of aborting the rest of the snapshot.
pub(crate) fn record_read_outcome<T>(
    coverage: &mut Vec<CoverageEntry>,
    category: ManifestCategoryName,
    endpoint: &str,
    outcome: actions::ReadOutcome<T>,
) -> Option<T> {
    match outcome {
        actions::ReadOutcome::Available(value) => Some(value),
        actions::ReadOutcome::NotApplicable(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::NotApplicable,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        actions::ReadOutcome::PermissionDenied(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::PermissionDenied,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        actions::ReadOutcome::Unavailable(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::Unavailable,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
    }
}
