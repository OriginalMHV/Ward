//! Secret resolution, redaction, and sealing shared by actions and environments.

use std::fmt;

use anyhow::Result;

use super::issue::ReconcileIssue;
use crate::config::manifest::{ExternalValueReference, SecretPlaceholderConfig};
use crate::github::actions;

/// A resolved secret plaintext value. `Debug` always redacts; the plaintext
/// is only ever exposed to the sealed-box encryption call at apply time.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn expose_for_encryption(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(REDACTED)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSecret {
    pub name: String,
    pub value: SecretValue,
}

/// Looks up an environment variable by name. Production code passes [`process_env`].
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Read a variable from the process environment. Non-Unicode values count as unset.
#[allow(
    clippy::disallowed_methods,
    reason = "the single production env lookup; everything else takes an injected lookup"
)]
pub fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Resolve a [`ExternalValueReference`] to a plaintext value. Returns `Err`
/// with a safe (non-sensitive) reason string on failure; the reason never
/// contains any resolved value.
pub(crate) fn resolve_external_value_with(
    reference: &ExternalValueReference,
    lookup: EnvLookup<'_>,
) -> Result<SecretValue, String> {
    match reference {
        ExternalValueReference::Env { key } => lookup(key)
            .map(SecretValue)
            .ok_or_else(|| format!("environment variable `{key}` is not set")),
        ExternalValueReference::Manual { hint } => Err(match hint {
            Some(hint) => format!("value must be provided manually ({hint})"),
            None => "value must be provided manually".to_owned(),
        }),
    }
}

pub(crate) fn resolve_secrets(
    placeholders: &[SecretPlaceholderConfig],
    scope_prefix: &str,
    issues: &mut Vec<ReconcileIssue>,
    lookup: EnvLookup<'_>,
) -> Vec<ResolvedSecret> {
    let mut resolved = Vec::new();
    for placeholder in placeholders {
        match resolve_external_value_with(&placeholder.value_from, lookup) {
            Ok(value) => resolved.push(ResolvedSecret {
                name: placeholder.name.clone(),
                value,
            }),
            Err(reason) => issues.push(ReconcileIssue::blocker(
                format!("{scope_prefix}.secrets.{}", placeholder.name),
                format!("Cannot resolve secret `{}`: {reason}", placeholder.name),
            )),
        }
    }
    resolved
}

/// Fetch a public key and seal `value` for it, converting encryption
/// failures into a blocked-action reason rather than propagating a hard
/// error (the plaintext is still never included in the reason).
pub(crate) fn seal_or_block(
    public_key: &str,
    name: &str,
    value: &SecretValue,
) -> Result<String, String> {
    actions::seal_secret_value(public_key, value.expose_for_encryption())
        .map_err(|_| format!("Failed to encrypt secret `{name}` with the target public key"))
}
