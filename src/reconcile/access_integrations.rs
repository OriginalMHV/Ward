//! Access and integration snapshot/reconciliation.

pub use super::access::*;
pub use super::integrations::*;

use crate::config::manifest::{CoverageEntry, CoverageOutcome, ManifestCategoryName};
use crate::github::actions::ReadOutcome;

pub(super) fn record_read_outcome<T>(
    coverage: &mut Vec<CoverageEntry>,
    category: ManifestCategoryName,
    endpoint: &str,
    outcome: ReadOutcome<T>,
) -> Option<T> {
    match outcome {
        ReadOutcome::Available(value) => Some(value),
        ReadOutcome::NotApplicable(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::NotApplicable,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        ReadOutcome::PermissionDenied(reason) => {
            coverage.push(CoverageEntry {
                category,
                endpoint: endpoint.to_owned(),
                outcome: CoverageOutcome::PermissionDenied,
                reason: Some(reason),
                required_permission: None,
            });
            None
        }
        ReadOutcome::Unavailable(reason) => {
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
