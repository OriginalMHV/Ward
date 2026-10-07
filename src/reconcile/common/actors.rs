//! Actor reference normalization and repository role lookup shared by rulesets and branch protection.

use std::collections::HashMap;

use crate::config::manifest::ActorReference;
use crate::github::rulesets::RulesetCustomRepositoryRole;

pub(crate) fn normalize_actor_refs(values: &[ActorReference]) -> Vec<String> {
    let mut normalized = values.iter().map(actor_reference_key).collect::<Vec<_>>();
    normalized.sort();
    normalized
}

pub(crate) fn actor_reference_key(actor: &ActorReference) -> String {
    match actor {
        ActorReference::OrganizationAdmin => "org-admin".to_owned(),
        ActorReference::Team { slug } => format!("team:{slug}"),
        ActorReference::User { login } => format!("user:{login}"),
        ActorReference::App { slug } => format!("app:{slug}"),
        ActorReference::Role { name } => format!("role:{name}"),
        ActorReference::Unresolved {
            actor_type,
            actor_id,
        } => {
            format!("unresolved:{actor_type}:{}", actor_id.unwrap_or_default())
        }
    }
}

pub(crate) fn repository_role_lookup(
    custom_roles: &[RulesetCustomRepositoryRole],
) -> HashMap<u64, String> {
    let mut roles = HashMap::from([
        (1, "read".to_owned()),
        (2, "maintain".to_owned()),
        (3, "triage".to_owned()),
        (4, "write".to_owned()),
        (5, "admin".to_owned()),
    ]);
    for role in custom_roles {
        roles.insert(role.id, role.name.clone());
    }
    roles
}
