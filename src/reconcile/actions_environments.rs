//! Snapshot and reconciliation for `ActionsCategory` and `EnvironmentsCategory`.
//!
//! Follows the collect/plan/apply/verify shape used by sibling reconcile
//! modules: `collect_*` observes live GitHub state (scoped to what `desired`
//! asks about, where enumerating "everything" would otherwise be unbounded),
//! `plan_*` is a pure/sync diff against that observation, `apply_*` executes
//! the plan, and `verify_*` re-collects and re-plans to confirm convergence.
//!
//! Secret values are never logged or persisted in plaintext: GitHub never
//! returns secret values, so collected secrets are represented as
//! placeholder entries; resolved plaintext lives only in [`SecretValue`],
//! whose `Debug` impl always redacts.

pub use super::actions::*;
pub use super::common::issue::{IssueSeverity, ReconcileIssue};
pub use super::common::secrets::{EnvLookup, ResolvedSecret, SecretValue, process_env};
pub use super::environments::*;
