use std::collections::HashMap;

use anyhow::Result;
use clap::Args;
use console::style;

use crate::github::Client;
use crate::github::branch_protection::BranchProtectionState;
use crate::github::repos::Repository;
use crate::github::security::SecurityState;

#[derive(Args)]
pub struct ImportCommand {
    /// GitHub organization to import from
    #[arg(long, required = true)]
    org: String,

    /// Output to stdout instead of ward.toml
    #[arg(long)]
    stdout: bool,

    /// Minimum repos to form a system (default: 2)
    #[arg(long, default_value_t = 2)]
    min_group_size: usize,

    /// Max concurrent API calls
    #[arg(long, default_value_t = 5)]
    parallelism: usize,
}

#[derive(Debug, Clone)]
struct DetectedSystem {
    id: String,
    repos: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct SampledSecurity {
    secret_scanning: bool,
    push_protection: bool,
    dependabot_alerts: bool,
    dependabot_security_updates: bool,
    secret_scanning_ai_detection: bool,
}

#[derive(Debug, Clone, Default)]
struct SampledProtection {
    enabled: bool,
    required_approvals: u32,
    dismiss_stale_reviews: bool,
    require_code_owner_reviews: bool,
    require_status_checks: bool,
    strict_status_checks: bool,
    enforce_admins: bool,
    required_linear_history: bool,
    allow_force_pushes: bool,
    allow_deletions: bool,
}

impl ImportCommand {
    pub async fn run(self) -> Result<()> {
        let client = Client::new(&self.org, self.parallelism).await?;

        println!(
            "\n  {} Fetching repositories for {}...",
            style("[..]").dim(),
            style(&self.org).cyan().bold()
        );

        let repos = client.list_repos().await?;
        let active: Vec<&Repository> = repos.iter().filter(|r| !r.archived).collect();

        println!(
            "  {} Found {} repositories ({} active)",
            style("[ok]").green(),
            repos.len(),
            active.len()
        );

        let active_names: Vec<String> = active.iter().map(|r| r.name.clone()).collect();
        let systems = detect_systems(&active_names, self.min_group_size);

        println!(
            "  {} Detected {} systems",
            style("[ok]").green(),
            systems.len()
        );
        for sys in &systems {
            println!(
                "    - {} ({} repos)",
                style(&sys.id).bold(),
                sys.repos.len()
            );
        }

        let grouped: Vec<&str> = systems
            .iter()
            .flat_map(|s| s.repos.iter().map(String::as_str))
            .collect();
        let ungrouped: Vec<&str> = active_names
            .iter()
            .filter(|n| !grouped.contains(&n.as_str()))
            .map(String::as_str)
            .collect();

        println!(
            "\n  {} Sampling security and branch protection...",
            style("[..]").dim()
        );

        let repo_map: HashMap<&str, &Repository> =
            active.iter().map(|r| (r.name.as_str(), *r)).collect();

        let mut system_security: HashMap<String, SampledSecurity> = HashMap::new();
        let mut global_protection = SampledProtection::default();
        let mut sampled_any_protection = false;

        for sys in &systems {
            let sample: Vec<&str> = sys.repos.iter().take(5).map(String::as_str).collect();
            let mut sec_states = Vec::new();
            let mut prot_states = Vec::new();

            for repo_name in &sample {
                if let Ok(sec) = client.get_security_state(repo_name).await {
                    sec_states.push(sec);
                }
                if let Some(repo) = repo_map.get(repo_name) {
                    if let Ok(Some(prot)) = client
                        .get_branch_protection(repo_name, &repo.default_branch)
                        .await
                    {
                        prot_states.push(prot);
                    }
                }
            }

            if !sec_states.is_empty() {
                system_security.insert(sys.id.clone(), majority_vote_security(&sec_states));
            }

            if !prot_states.is_empty() && !sampled_any_protection {
                global_protection = majority_vote_protection(&prot_states);
                sampled_any_protection = true;
            }
        }

        let global_sec = if system_security.is_empty() {
            SampledSecurity::default()
        } else {
            merge_security_samples(system_security.values())
        };

        let team_map = sample_teams(&client, &systems).await;

        let toml_output = generate_toml(
            &self.org,
            &systems,
            &ungrouped,
            &global_sec,
            &global_protection,
            sampled_any_protection,
            &team_map,
        );

        if self.stdout {
            println!("{toml_output}");
        } else {
            let path = "ward.toml";
            if std::path::Path::new(path).exists() {
                anyhow::bail!(
                    "ward.toml already exists. Use --stdout to print instead, or remove the file first."
                );
            }
            std::fs::write(path, &toml_output)?;
            println!("\n  {} Wrote {}", style("[ok]").green(), style(path).bold());
        }

        println!(
            "\n  {} Import complete. Review the generated config and adjust as needed.",
            style("[ok]").green()
        );

        Ok(())
    }
}

fn detect_systems(repo_names: &[String], min_group_size: usize) -> Vec<DetectedSystem> {
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();

    for name in repo_names {
        if let Some(prefix) = name.split('-').next() {
            if !prefix.is_empty() && prefix != name {
                groups
                    .entry(prefix.to_string())
                    .or_default()
                    .push(name.clone());
            }
        }
    }

    let mut systems: Vec<DetectedSystem> = groups
        .into_iter()
        .filter(|(_, repos)| repos.len() >= min_group_size)
        .map(|(id, mut repos)| {
            repos.sort();
            DetectedSystem { id, repos }
        })
        .collect();

    systems.sort_by(|a, b| a.id.cmp(&b.id));
    systems
}

fn majority_vote_security(states: &[SecurityState]) -> SampledSecurity {
    let n = states.len();
    let threshold = n / 2 + 1;

    SampledSecurity {
        secret_scanning: states.iter().filter(|s| s.secret_scanning).count() >= threshold,
        push_protection: states.iter().filter(|s| s.push_protection).count() >= threshold,
        dependabot_alerts: states.iter().filter(|s| s.dependabot_alerts).count() >= threshold,
        dependabot_security_updates: states
            .iter()
            .filter(|s| s.dependabot_security_updates)
            .count()
            >= threshold,
        secret_scanning_ai_detection: states
            .iter()
            .filter(|s| s.secret_scanning_ai_detection)
            .count()
            >= threshold,
    }
}

fn majority_vote_protection(states: &[BranchProtectionState]) -> SampledProtection {
    let n = states.len();
    let threshold = n / 2 + 1;

    let approvals: Vec<u32> = states
        .iter()
        .map(|s| s.required_approving_review_count)
        .collect();
    let median_approvals = {
        let mut sorted = approvals.clone();
        sorted.sort();
        sorted[sorted.len() / 2]
    };

    SampledProtection {
        enabled: states
            .iter()
            .filter(|s| s.required_pull_request_reviews)
            .count()
            >= threshold,
        required_approvals: median_approvals,
        dismiss_stale_reviews: states.iter().filter(|s| s.dismiss_stale_reviews).count()
            >= threshold,
        require_code_owner_reviews: states
            .iter()
            .filter(|s| s.require_code_owner_reviews)
            .count()
            >= threshold,
        require_status_checks: states.iter().filter(|s| s.required_status_checks).count()
            >= threshold,
        strict_status_checks: states.iter().filter(|s| s.strict_status_checks).count() >= threshold,
        enforce_admins: states.iter().filter(|s| s.enforce_admins).count() >= threshold,
        required_linear_history: states.iter().filter(|s| s.required_linear_history).count()
            >= threshold,
        allow_force_pushes: states.iter().filter(|s| s.allow_force_pushes).count() >= threshold,
        allow_deletions: states.iter().filter(|s| s.allow_deletions).count() >= threshold,
    }
}

fn merge_security_samples<'a>(
    samples: impl Iterator<Item = &'a SampledSecurity>,
) -> SampledSecurity {
    let all: Vec<&SampledSecurity> = samples.collect();
    let n = all.len();
    let threshold = n / 2 + 1;

    SampledSecurity {
        secret_scanning: all.iter().filter(|s| s.secret_scanning).count() >= threshold,
        push_protection: all.iter().filter(|s| s.push_protection).count() >= threshold,
        dependabot_alerts: all.iter().filter(|s| s.dependabot_alerts).count() >= threshold,
        dependabot_security_updates: all.iter().filter(|s| s.dependabot_security_updates).count()
            >= threshold,
        secret_scanning_ai_detection: all
            .iter()
            .filter(|s| s.secret_scanning_ai_detection)
            .count()
            >= threshold,
    }
}

async fn sample_teams(
    client: &Client,
    systems: &[DetectedSystem],
) -> HashMap<String, Vec<(String, String)>> {
    let mut team_map: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for sys in systems {
        if let Some(repo_name) = sys.repos.first() {
            if let Ok(teams) = client.list_repo_teams(repo_name).await {
                let entries: Vec<(String, String)> =
                    teams.into_iter().map(|t| (t.slug, t.permission)).collect();
                if !entries.is_empty() {
                    team_map.insert(sys.id.clone(), entries);
                }
            }
        }
    }

    team_map
}

fn generate_toml(
    org: &str,
    systems: &[DetectedSystem],
    ungrouped: &[&str],
    security: &SampledSecurity,
    protection: &SampledProtection,
    has_protection: bool,
    team_map: &HashMap<String, Vec<(String, String)>>,
) -> String {
    let mut out = String::new();

    out.push_str(&format!("# Ward configuration -- imported from {org}\n\n"));

    out.push_str(&format!("[org]\nname = \"{org}\"\n\n"));

    out.push_str("# Security settings (sampled from existing repos)\n");
    out.push_str("[security]\n");
    out.push_str(&format!("secret_scanning = {}\n", security.secret_scanning));
    out.push_str(&format!(
        "secret_scanning_ai_detection = {}\n",
        security.secret_scanning_ai_detection
    ));
    out.push_str(&format!("push_protection = {}\n", security.push_protection));
    out.push_str(&format!(
        "dependabot_alerts = {}\n",
        security.dependabot_alerts
    ));
    out.push_str(&format!(
        "dependabot_security_updates = {}\n",
        security.dependabot_security_updates
    ));
    out.push('\n');

    if has_protection {
        out.push_str("# Branch protection (sampled from existing repos)\n");
        out.push_str("[branch_protection]\n");
        out.push_str(&format!("enabled = {}\n", protection.enabled));
        out.push_str(&format!(
            "required_approvals = {}\n",
            protection.required_approvals
        ));
        out.push_str(&format!(
            "dismiss_stale_reviews = {}\n",
            protection.dismiss_stale_reviews
        ));
        out.push_str(&format!(
            "require_code_owner_reviews = {}\n",
            protection.require_code_owner_reviews
        ));
        out.push_str(&format!(
            "require_status_checks = {}\n",
            protection.require_status_checks
        ));
        out.push_str(&format!(
            "strict_status_checks = {}\n",
            protection.strict_status_checks
        ));
        out.push_str(&format!("enforce_admins = {}\n", protection.enforce_admins));
        out.push_str(&format!(
            "required_linear_history = {}\n",
            protection.required_linear_history
        ));
        out.push_str(&format!(
            "allow_force_pushes = {}\n",
            protection.allow_force_pushes
        ));
        out.push_str(&format!(
            "allow_deletions = {}\n",
            protection.allow_deletions
        ));
        out.push('\n');
    }

    for sys in systems {
        out.push_str(&format!("# Detected system: {} repos\n", sys.repos.len()));
        out.push_str("[[systems]]\n");
        out.push_str(&format!("id = \"{}\"\n", sys.id));
        out.push_str(&format!("name = \"{}\"\n", titlecase(&sys.id)));

        if let Some(teams) = team_map.get(&sys.id) {
            out.push_str("teams = [\n");
            for (slug, perm) in teams {
                out.push_str(&format!(
                    "    {{ slug = \"{slug}\", permission = \"{perm}\" }},\n"
                ));
            }
            out.push_str("]\n");
        }

        out.push('\n');
    }

    if !ungrouped.is_empty() {
        out.push_str("# Ungrouped repositories (did not match any system prefix)\n");
        for name in ungrouped {
            out.push_str(&format!("# - {name}\n"));
        }
        out.push('\n');
    }

    out
}

fn titlecase(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_systems_groups_by_prefix() {
        let repos = vec![
            "backend-api".to_string(),
            "backend-auth".to_string(),
            "backend-common".to_string(),
            "frontend-web".to_string(),
            "frontend-mobile".to_string(),
            "standalone".to_string(),
        ];

        let systems = detect_systems(&repos, 2);
        assert_eq!(systems.len(), 2);

        let be = systems.iter().find(|s| s.id == "backend").unwrap();
        assert_eq!(be.repos.len(), 3);
        assert!(be.repos.contains(&"backend-api".to_string()));
        assert!(be.repos.contains(&"backend-auth".to_string()));
        assert!(be.repos.contains(&"backend-common".to_string()));

        let fe = systems.iter().find(|s| s.id == "frontend").unwrap();
        assert_eq!(fe.repos.len(), 2);
    }

    #[test]
    fn test_detect_systems_respects_min_group_size() {
        let repos = vec![
            "backend-api".to_string(),
            "backend-auth".to_string(),
            "frontend-web".to_string(),
        ];

        let systems_min2 = detect_systems(&repos, 2);
        assert_eq!(systems_min2.len(), 1);
        assert_eq!(systems_min2[0].id, "backend");

        let systems_min3 = detect_systems(&repos, 3);
        assert!(systems_min3.is_empty());
    }

    #[test]
    fn test_majority_vote_security() {
        let states = vec![
            SecurityState {
                secret_scanning: true,
                push_protection: true,
                dependabot_alerts: true,
                dependabot_security_updates: false,
                secret_scanning_ai_detection: false,
            },
            SecurityState {
                secret_scanning: true,
                push_protection: false,
                dependabot_alerts: true,
                dependabot_security_updates: false,
                secret_scanning_ai_detection: true,
            },
            SecurityState {
                secret_scanning: true,
                push_protection: true,
                dependabot_alerts: false,
                dependabot_security_updates: false,
                secret_scanning_ai_detection: false,
            },
        ];

        let result = majority_vote_security(&states);
        assert!(result.secret_scanning); // 3/3
        assert!(result.push_protection); // 2/3
        assert!(result.dependabot_alerts); // 2/3
        assert!(!result.dependabot_security_updates); // 0/3
        assert!(!result.secret_scanning_ai_detection); // 1/3
    }

    #[test]
    fn test_generate_toml_output() {
        let systems = vec![DetectedSystem {
            id: "backend".to_string(),
            repos: vec!["backend-api".to_string(), "backend-auth".to_string()],
        }];
        let ungrouped: Vec<&str> = vec!["standalone"];
        let security = SampledSecurity {
            secret_scanning: true,
            push_protection: true,
            dependabot_alerts: true,
            dependabot_security_updates: false,
            secret_scanning_ai_detection: false,
        };
        let protection = SampledProtection {
            enabled: true,
            required_approvals: 1,
            ..Default::default()
        };
        let team_map = HashMap::new();

        let toml = generate_toml(
            "my-org",
            &systems,
            &ungrouped,
            &security,
            &protection,
            true,
            &team_map,
        );

        assert!(toml.contains("[org]"));
        assert!(toml.contains("name = \"my-org\""));
        assert!(toml.contains("secret_scanning = true"));
        assert!(toml.contains("dependabot_security_updates = false"));
        assert!(toml.contains("[[systems]]"));
        assert!(toml.contains("id = \"backend\""));
        assert!(toml.contains("enabled = true"));
        assert!(toml.contains("required_approvals = 1"));
        assert!(toml.contains("# - standalone"));
    }

    #[test]
    fn test_detect_systems_excludes_single_segment_names() {
        let repos = vec![
            "standalone".to_string(),
            "another".to_string(),
            "third".to_string(),
        ];
        let systems = detect_systems(&repos, 2);
        assert!(systems.is_empty());
    }

    #[test]
    fn test_majority_vote_protection() {
        let states = vec![
            BranchProtectionState {
                required_pull_request_reviews: true,
                required_approving_review_count: 2,
                dismiss_stale_reviews: true,
                ..Default::default()
            },
            BranchProtectionState {
                required_pull_request_reviews: true,
                required_approving_review_count: 1,
                dismiss_stale_reviews: false,
                ..Default::default()
            },
            BranchProtectionState {
                required_pull_request_reviews: false,
                required_approving_review_count: 1,
                dismiss_stale_reviews: true,
                ..Default::default()
            },
        ];

        let result = majority_vote_protection(&states);
        assert!(result.enabled); // 2/3
        assert_eq!(result.required_approvals, 1); // median
        assert!(result.dismiss_stale_reviews); // 2/3
    }
}
