use anyhow::Result;
use serde::de::DeserializeOwned;

use super::response::{self, ClassifiedResponse};

/// The outcome of a write (create/update/delete) call to a GitHub endpoint
/// whose failure modes include expected, non-fatal conditions (validation
/// errors, organization-locked settings, or endpoints that do not apply to a
/// given repository). `Blocked` carries a redacted, human-readable reason
/// (GitHub response bodies are never included verbatim; see
/// [`super::error::ApiFailure`]'s `Display` impl) so callers can
/// report it without treating it as a hard failure of the whole run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome<T = ()> {
    Applied(T),
    Blocked(String),
}

/// Classify a write response with no expected body: 2xx/204 is `Applied`,
/// 403/404/422/other is `Blocked`. Transport-level failures still propagate
/// as `Err`.
pub(crate) async fn write_empty(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<WriteOutcome> {
    match response::classify_empty(response, method, path).await? {
        ClassifiedResponse::Success(()) | ClassifiedResponse::NoContent => {
            Ok(WriteOutcome::Applied(()))
        }
        ClassifiedResponse::NotFound(error)
        | ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Ok(WriteOutcome::Blocked(error.to_string())),
    }
}

/// As [`write_empty`], but a 404 is treated as `Applied` since deleting an
/// already-absent resource is an idempotent no-op, not a blocked action.
pub(crate) async fn write_delete(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<WriteOutcome> {
    match response::classify_empty(response, method, path).await? {
        ClassifiedResponse::Success(())
        | ClassifiedResponse::NoContent
        | ClassifiedResponse::NotFound(_) => Ok(WriteOutcome::Applied(())),
        ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Ok(WriteOutcome::Blocked(error.to_string())),
    }
}

/// The outcome of a *read* against an optional or coverage-tracked GitHub
/// endpoint. Import/collection must never abort because one sub-endpoint is
/// permission-restricted, not applicable to a given repository's visibility,
/// or otherwise unavailable: every caller matches on this instead of using
/// `?` directly, so it can always record a `CoverageEntry` and continue with
/// the rest of collection. Transport-level failures (network errors, JSON
/// decode errors) still propagate as `Err`, since those are genuine
/// anomalies rather than expected per-endpoint conditions.
#[derive(Debug, Clone)]
pub enum ReadOutcome<T> {
    /// The endpoint returned a usable value.
    Available(T),
    /// The endpoint does not apply to this repository (e.g. a private-only
    /// or public-only policy surface), observed as a 404 or 422 whose
    /// message indicates non-applicability rather than a real error.
    NotApplicable(String),
    /// The caller's token lacks the permission required to read this
    /// endpoint (403).
    PermissionDenied(String),
    /// The endpoint is unavailable for some other reason (plan restriction,
    /// unexpected status, etc.).
    Unavailable(String),
}

impl<T> ReadOutcome<T> {
    /// Returns the value if available, discarding the reason otherwise.
    pub fn available(self) -> Option<T> {
        match self {
            Self::Available(value) => Some(value),
            Self::NotApplicable(_) | Self::PermissionDenied(_) | Self::Unavailable(_) => None,
        }
    }
}

/// Classify a JSON read response into a [`ReadOutcome`]. `not_found_is_not_applicable`
/// controls whether a 404 is treated as "this repository doesn't support the
/// endpoint" (`NotApplicable`) or as a genuine unavailability (`Unavailable`);
/// a 422 is always treated as `NotApplicable`, matching GitHub's documented
/// and observed behavior of returning 422 for endpoints that are valid in
/// shape but do not apply to the target repository (e.g. fork PR contributor
/// approval on a private repository).
pub(crate) async fn classify_read<T>(
    response: reqwest::Response,
    method: &str,
    path: &str,
    not_found_is_not_applicable: bool,
) -> Result<ReadOutcome<T>>
where
    T: DeserializeOwned,
{
    match response::classify_json(response, method, path).await? {
        ClassifiedResponse::Success(value) => Ok(ReadOutcome::Available(value)),
        ClassifiedResponse::NoContent => Ok(ReadOutcome::Unavailable(format!(
            "{method} {path} returned HTTP 204 No Content when JSON was expected"
        ))),
        ClassifiedResponse::NotFound(error) => Ok(if not_found_is_not_applicable {
            ReadOutcome::NotApplicable(error.to_string())
        } else {
            ReadOutcome::Unavailable(error.to_string())
        }),
        ClassifiedResponse::Forbidden(error) => {
            Ok(ReadOutcome::PermissionDenied(error.to_string()))
        }
        ClassifiedResponse::Unprocessable(error) => {
            Ok(ReadOutcome::NotApplicable(error.to_string()))
        }
        ClassifiedResponse::Other(error) => Ok(ReadOutcome::Unavailable(error.to_string())),
    }
}

/// Classify a write response that returns the created/updated resource as
/// JSON on success.
pub(crate) async fn write_json<T>(
    response: reqwest::Response,
    method: &str,
    path: &str,
) -> Result<WriteOutcome<T>>
where
    T: DeserializeOwned,
{
    match response::classify_json(response, method, path).await? {
        ClassifiedResponse::Success(value) => Ok(WriteOutcome::Applied(value)),
        ClassifiedResponse::NoContent => Ok(WriteOutcome::Blocked(format!(
            "{method} {path} returned HTTP 204 No Content when a resource body was expected"
        ))),
        ClassifiedResponse::NotFound(error)
        | ClassifiedResponse::Forbidden(error)
        | ClassifiedResponse::Unprocessable(error)
        | ClassifiedResponse::Other(error) => Ok(WriteOutcome::Blocked(error.to_string())),
    }
}
