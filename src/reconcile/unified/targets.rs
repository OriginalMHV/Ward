use anyhow::{Context, Result, bail};

use crate::config::Manifest;
use crate::github::Client;
use crate::github::repos::Repository;

// ---------------------------------------------------------------------------
// Target repository resolution (same-owner, existing repos only)
// ---------------------------------------------------------------------------

/// Resolve the set of existing, same-owner repositories to reconcile.
/// The manifest's systems define the scope. `--system` and `--repo` only
/// narrow it, so an unknown system or an out-of-scope repository is an error.
/// Never creates repositories.
pub async fn resolve_target_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<Repository>> {
    let system_ids: Vec<String> = if let Some(system_id) = system {
        if manifest.system(system_id).is_none() {
            let known: Vec<&str> = manifest.systems.iter().map(|s| s.id.as_str()).collect();
            bail!(
                "Unknown system '{system_id}'. Configured systems: {}",
                if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                }
            );
        }
        vec![system_id.to_owned()]
    } else {
        manifest.systems.iter().map(|s| s.id.clone()).collect()
    };

    if system_ids.is_empty() && repo.is_none() {
        bail!(
            "No target selected. Pass --repo <name>, pass --system <id>, or configure systems in ward.toml"
        );
    }

    if let Some(repo_name) = repo {
        // Without systems the manifest has only top-level categories, so there is
        // no scope to escape and --repo is the explicit target.
        if !system_ids.is_empty() {
            let mut in_scope = false;
            for system_id in &system_ids {
                if system_includes_repo(manifest, system_id, repo_name)? {
                    in_scope = true;
                    break;
                }
            }
            if !in_scope {
                bail!(
                    "Repository '{repo_name}' is not in the manifest scope (systems: {}). Add it to a system or check its exclude patterns",
                    system_ids.join(", ")
                );
            }
        }
        return Ok(vec![client.get_repo(repo_name).await?]);
    }

    let mut repos: Vec<Repository> = Vec::new();
    for system_id in &system_ids {
        let excludes = manifest.exclude_patterns_for_system(system_id);
        let explicit = manifest.explicit_repos_for_system(system_id);
        let found = client
            .list_repos_for_system(
                system_id,
                manifest.matches_prefix_for_system(system_id),
                &excludes,
                &explicit,
            )
            .await?;
        for repository in found {
            if !repos
                .iter()
                .any(|existing| existing.name == repository.name)
            {
                repos.push(repository);
            }
        }
    }

    Ok(repos)
}

/// Whether a system selects the named repository: listed explicitly, or
/// matched by prefix and not excluded. Mirrors `Client::list_repos_for_system`.
fn system_includes_repo(manifest: &Manifest, system_id: &str, repo: &str) -> Result<bool> {
    if manifest
        .explicit_repos_for_system(system_id)
        .iter()
        .any(|explicit| explicit.eq_ignore_ascii_case(repo))
    {
        return Ok(true);
    }
    if !manifest.matches_prefix_for_system(system_id) {
        return Ok(false);
    }
    // GitHub repository names are case-insensitive.
    let repo = repo.to_ascii_lowercase();
    let prefix = system_id.to_ascii_lowercase();
    let suffix = match repo.strip_prefix(prefix.as_str()) {
        Some("") => repo.as_str(),
        Some(rest) => match rest.strip_prefix('-') {
            Some(suffix) => suffix,
            None => return Ok(false),
        },
        None => return Ok(false),
    };
    let excludes = manifest.exclude_patterns_for_system(system_id);
    if excludes.is_empty() {
        return Ok(true);
    }
    let pattern = regex::RegexBuilder::new(&excludes.join("|"))
        .case_insensitive(true)
        .build()
        .context("Invalid exclude pattern regex")?;
    Ok(!pattern.is_match(suffix))
}

/// Complete every read-only plan and dependency preflight before the first
/// mutation so a later repository cannot surprise a partially applied run.
/// An explicitly named archived repository is an error for apply. Archived
/// repositories found by a system scope are skipped by [`prepare_apply`] instead.
pub fn reject_archived_explicit_target(repos: &[Repository]) -> Result<()> {
    match repos.iter().find(|repository| repository.archived) {
        Some(archived) => bail!(
            "Repository '{}' is archived. Ward plans and audits archived repositories but does not apply changes to them",
            archived.name
        ),
        None => Ok(()),
    }
}
