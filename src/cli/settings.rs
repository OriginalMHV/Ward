use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::config::templates::load_templates_with_custom_dir;
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::github::commits::CommitFile;

#[derive(Args)]
pub struct SettingsCommand {
    #[command(subcommand)]
    action: SettingsAction,
}

#[derive(clap::Subcommand)]
enum SettingsAction {
    /// Show what settings/rulesets would change
    Plan {
        /// Ruleset to apply (copilot-review)
        #[arg(long)]
        ruleset: Option<String>,

        /// Deploy copilot review instructions
        #[arg(long)]
        copilot_instructions: bool,
    },

    /// Apply settings and rulesets
    Apply {
        /// Ruleset to apply (copilot-review)
        #[arg(long)]
        ruleset: Option<String>,

        /// Deploy copilot review instructions
        #[arg(long)]
        copilot_instructions: bool,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Audit current settings state
    Audit,
}

impl SettingsCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        match &self.action {
            SettingsAction::Plan {
                ruleset,
                copilot_instructions,
            } => {
                plan(
                    client,
                    manifest,
                    system,
                    repo,
                    ruleset.as_deref(),
                    *copilot_instructions,
                )
                .await
            }
            SettingsAction::Apply {
                ruleset,
                copilot_instructions,
                yes,
            } => {
                apply(
                    client,
                    manifest,
                    system,
                    repo,
                    ruleset.as_deref(),
                    *copilot_instructions,
                    *yes,
                )
                .await
            }
            SettingsAction::Audit => audit(client, manifest, system, repo).await,
        }
    }
}

/// Detect if a repo is an operations/GitOps repo (vs application repo).
fn is_ops_repo(repo_name: &str) -> bool {
    repo_name.ends_with("-operation")
        || repo_name.ends_with("-operations")
        || repo_name.ends_with("-ops")
        || repo_name.ends_with("-gitops")
}

struct RepoRulesetState {
    repo: String,
    has_copilot_review: bool,
    has_instructions: bool,
    is_ops: bool,
}

async fn scan_repo(client: &Client, repo: &str) -> Result<RepoRulesetState> {
    let rulesets = client.list_rulesets(repo).await?;
    let has_copilot_review = rulesets.iter().any(|r| r.name == "Copilot Code Review");

    let has_instructions = client
        .get_file(repo, ".github/copilot-instructions.md", None)
        .await?
        .is_some();

    Ok(RepoRulesetState {
        repo: repo.to_owned(),
        has_copilot_review,
        has_instructions,
        is_ops: is_ops_repo(repo),
    })
}

async fn resolve_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<String>> {
    if let Some(repo_name) = repo {
        return Ok(vec![repo_name.to_owned()]);
    }
    let sys = system.ok_or_else(|| anyhow::anyhow!("Either --system or --repo is required"))?;
    let excludes = manifest.exclude_patterns_for_system(sys);
    let explicit = manifest.explicit_repos_for_system(sys);
    let repos = client
        .list_repos_for_system(sys, &excludes, &explicit)
        .await?;
    Ok(repos.into_iter().map(|r| r.name).collect())
}

async fn plan(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    ruleset: Option<&str>,
    copilot_instructions: bool,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;
    let do_ruleset = ruleset.is_some() || (!copilot_instructions);
    let do_instructions = copilot_instructions || ruleset.is_none();

    println!();
    println!(
        "  {} Settings plan: scanning {} repos...",
        style("🔍").bold(),
        repos.len()
    );
    println!();

    let mut ruleset_needed = 0;
    let mut instructions_needed = 0;
    let mut up_to_date = 0;

    for repo_name in &repos {
        let state = scan_repo(client, repo_name).await?;
        let mut changes = Vec::new();

        if do_ruleset && !state.has_copilot_review {
            changes.push("create Copilot Code Review ruleset");
            ruleset_needed += 1;
        }

        if do_instructions && !state.has_instructions {
            changes.push(if state.is_ops {
                "deploy copilot-instructions.md (ops)"
            } else {
                "deploy copilot-instructions.md (app)"
            });
            instructions_needed += 1;
        }

        if changes.is_empty() {
            println!("  {} {}", style("✓").green(), style(repo_name).dim());
            up_to_date += 1;
        } else {
            println!("  {} {}", style("⚡").yellow(), style(repo_name).bold());
            for change in &changes {
                println!("     {change}");
            }
        }
    }

    println!();
    println!(
        "  Summary: {} need ruleset, {} need instructions, {} up to date",
        style(ruleset_needed).yellow().bold(),
        style(instructions_needed).yellow().bold(),
        style(up_to_date).green()
    );

    if ruleset_needed + instructions_needed > 0 {
        println!(
            "\n  Run {} to apply.",
            style("ward settings apply").cyan().bold()
        );
    }

    Ok(())
}

async fn apply(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    ruleset: Option<&str>,
    copilot_instructions: bool,
    yes: bool,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;
    let do_ruleset = ruleset.is_some() || (!copilot_instructions);
    let do_instructions = copilot_instructions || ruleset.is_none();
    let branch_name = &manifest.templates.branch;

    println!();
    println!("  {} Scanning {} repos...", style("🔍").bold(), repos.len());

    // Scan all repos
    let mut work: Vec<(RepoRulesetState, String)> = Vec::new();
    for repo_name in &repos {
        let state = scan_repo(client, repo_name).await?;
        let r = client.get_repo(repo_name).await?;
        let needs_work = (do_ruleset && !state.has_copilot_review)
            || (do_instructions && !state.has_instructions);
        if needs_work {
            work.push((state, r.default_branch));
        }
    }

    if work.is_empty() {
        println!("\n  {} All repos up to date.", style("✅").green());
        return Ok(());
    }

    println!(
        "\n  {} repos need changes:",
        style(work.len()).yellow().bold()
    );
    for (state, _) in &work {
        let mut actions = Vec::new();
        if do_ruleset && !state.has_copilot_review {
            actions.push("ruleset");
        }
        if do_instructions && !state.has_instructions {
            actions.push(if state.is_ops {
                "instructions (ops)"
            } else {
                "instructions (app)"
            });
        }
        println!(
            "  {} {} - {}",
            style("⚡").yellow(),
            state.repo,
            actions.join(", ")
        );
    }

    if !yes {
        println!();
        let proceed = Confirm::new()
            .with_prompt(format!("  Apply to {} repos?", work.len()))
            .default(false)
            .interact()?;
        if !proceed {
            println!("  Aborted.");
            return Ok(());
        }
    }

    let audit_log = AuditLog::new()?;
    let tera = load_templates_with_custom_dir(
        manifest
            .templates
            .custom_dir
            .as_ref()
            .map(std::path::Path::new),
    )?;
    let mut succeeded = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    for (state, default_branch) in &work {
        println!("  {} {} ...", style("▶").magenta(), state.repo);

        // Create ruleset
        if do_ruleset && !state.has_copilot_review {
            match client.create_copilot_review_ruleset(&state.repo).await {
                Ok(()) => {
                    println!("    {} Copilot review ruleset created", style("✅").green());
                    audit_log.log(
                        &state.repo,
                        "create_copilot_review_ruleset",
                        "success",
                        false,
                        true,
                    )?;
                }
                Err(e) => {
                    println!("    {} Ruleset: {e}", style("❌").red());
                    failed.push((state.repo.clone(), format!("ruleset: {e}")));
                    continue;
                }
            }
        }

        // Deploy instructions
        if do_instructions && !state.has_instructions {
            let template_name = if state.is_ops {
                "copilot-review/instructions-ops.md.tera"
            } else {
                "copilot-review/instructions-app.md.tera"
            };

            let ctx = tera::Context::new();
            match tera.render(template_name, &ctx) {
                Ok(rendered) => {
                    match deploy_instructions(
                        client,
                        &state.repo,
                        default_branch,
                        branch_name,
                        &rendered,
                        &manifest.templates.reviewers,
                        &manifest.templates.commit_message_prefix,
                    )
                    .await
                    {
                        Ok(pr_url) => {
                            println!(
                                "    {} Instructions PR: {}",
                                style("✅").green(),
                                style(&pr_url).cyan()
                            );
                            audit_log.log(
                                &state.repo,
                                "deploy_copilot_instructions",
                                "success",
                                false,
                                true,
                            )?;
                        }
                        Err(e) => {
                            println!("    {} Instructions: {e}", style("❌").red());
                            failed.push((state.repo.clone(), format!("instructions: {e}")));
                            continue;
                        }
                    }
                }
                Err(e) => {
                    println!("    {} Template render: {e}", style("❌").red());
                    failed.push((state.repo.clone(), format!("template: {e}")));
                    continue;
                }
            }
        }

        succeeded += 1;
    }

    println!();
    if failed.is_empty() {
        println!("  {} All {} repos updated.", style("✅").green(), succeeded);
    } else {
        println!(
            "  {} {} succeeded, {} failed:",
            style("⚠️").yellow(),
            succeeded,
            failed.len()
        );
        for (repo, err) in &failed {
            println!("    {} {}: {}", style("❌").red(), repo, err);
        }
    }

    println!(
        "\n  {} Audit log: {}",
        style("📋").bold(),
        audit_log.path().display()
    );

    Ok(())
}

async fn deploy_instructions(
    client: &Client,
    repo: &str,
    default_branch: &str,
    branch_name: &str,
    content: &str,
    reviewers: &[String],
    commit_prefix: &str,
) -> Result<String> {
    client
        .create_branch(repo, branch_name, default_branch)
        .await?;

    let files = vec![CommitFile {
        path: ".github/copilot-instructions.md".to_owned(),
        content: content.to_owned(),
    }];

    client
        .create_commit(
            repo,
            branch_name,
            &format!("{commit_prefix}add Copilot review instructions"),
            &files,
        )
        .await?;

    let pr = client
        .create_pull_request(
            repo,
            &format!("{commit_prefix}add Copilot review instructions"),
            "## Ward: Copilot review instructions\n\n\
             Deploys `.github/copilot-instructions.md` for automatic Copilot code review.\n\n\
             ---\n\
             *Review the instructions, then merge.*",
            branch_name,
            default_branch,
            reviewers,
        )
        .await?;

    Ok(pr.html_url)
}

async fn audit(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Settings audit: {} repos",
        style("🔍").bold(),
        repos.len()
    );
    println!();
    println!(
        "  {:40} {:10} {:14} {}",
        style("Repository").bold().underlined(),
        style("Type").bold().underlined(),
        style("Review Rule").bold().underlined(),
        style("Instructions").bold().underlined(),
    );

    let mut all_ok = 0;
    let mut issues = 0;

    for repo_name in &repos {
        let state = scan_repo(client, repo_name).await?;

        let ruleset_icon = if state.has_copilot_review {
            format!("{}", style("✅").green())
        } else {
            format!("{}", style("❌").red())
        };
        let instr_icon = if state.has_instructions {
            format!("{}", style("✅").green())
        } else {
            format!("{}", style("❌").red())
        };
        let repo_type = if state.is_ops { "ops" } else { "app" };

        let ok = state.has_copilot_review && state.has_instructions;
        if ok {
            all_ok += 1;
        } else {
            issues += 1;
        }

        println!(
            "  {:40} {:10} {:14} {}",
            repo_name, repo_type, ruleset_icon, instr_icon
        );
    }

    println!();
    println!(
        "  Summary: {} fully configured, {} need attention",
        style(all_ok).green().bold(),
        if issues > 0 {
            style(issues).red().bold()
        } else {
            style(issues).green().bold()
        }
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_ops_repo_by_operations_suffix() {
        assert!(is_ops_repo("backend-user-service-operations"));
    }

    #[test]
    fn detect_ops_repo_by_operation_singular() {
        assert!(is_ops_repo("backend-user-service-operation"));
    }

    #[test]
    fn detect_ops_repo_by_ops_suffix() {
        assert!(is_ops_repo("frontend-app-ops"));
    }

    #[test]
    fn detect_ops_repo_by_gitops_suffix() {
        assert!(is_ops_repo("platform-gitops"));
    }

    #[test]
    fn detect_ops_repo_with_operation_in_middle() {
        assert!(!is_ops_repo("my-operation-manager"));
    }

    #[test]
    fn detect_ops_repo_by_operation_suffix() {
        assert!(is_ops_repo("my-service-operation"));
    }

    #[test]
    fn regular_repo_not_ops() {
        assert!(!is_ops_repo("backend-user-service"));
    }

    #[test]
    fn regular_repo_with_similar_name() {
        assert!(!is_ops_repo("backend-optimizer"));
    }
}
