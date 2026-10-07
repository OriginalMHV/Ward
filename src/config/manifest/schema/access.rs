use serde::{Deserialize, Serialize};

use super::{ActorReference, CategoryPolicy, ReferencedResourceConfig};
use crate::config::manifest::TeamAccess;

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryAccessCategory {
    #[serde(default)]
    pub policy: CategoryPolicy,

    /// Team access. `None` (the key is absent) means teams are not managed and
    /// are never removed. `Some(vec![])` is an explicit empty list: with
    /// `prune = true` it removes every team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teams: Option<Vec<TeamAccess>>,

    /// Collaborator access. `None` (the key is absent) means collaborators are
    /// not managed and are never removed. `Some(vec![])` is an explicit empty
    /// list: with `prune = true` it removes every collaborator and invitation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaborators: Option<Vec<CollaboratorAccessConfig>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ReferencedResourceConfig>,
}

impl RepositoryAccessCategory {
    /// The desired teams, or an empty slice when the manifest does not manage teams.
    pub fn desired_teams(&self) -> &[TeamAccess] {
        self.teams.as_deref().unwrap_or_default()
    }

    /// The desired collaborators, or an empty slice when the manifest does not manage them.
    pub fn desired_collaborators(&self) -> &[CollaboratorAccessConfig] {
        self.collaborators.as_deref().unwrap_or_default()
    }

    pub fn observe_sensitive() -> Self {
        Self {
            policy: CategoryPolicy::observe_sensitive(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollaboratorAccessConfig {
    pub actor: ActorReference,
    pub permission: String,
}
