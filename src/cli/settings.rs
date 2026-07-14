use std::fmt;

use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::config::manifest::RepositorySettingsConfig;
use crate::config::templates::load_templates_with_custom_dir;
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::github::commits::CommitFile;
use crate::github::settings::RepoSettings;

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
                crate::reconcile::unified::guard_legacy_mutation(
                    manifest,
                    crate::reconcile::unified::Category::Repository,
                    "settings apply",
                )?;
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
    repository_changes: Vec<RepositorySettingChange>,
}

#[derive(Debug)]
struct RepositorySettingChange {
    field: &'static str,
    current: String,
    desired: String,
}

async fn scan_repo(
    client: &Client,
    repo: &str,
    desired_repository: Option<&RepositorySettingsConfig>,
    check_copilot_review: bool,
    check_instructions: bool,
) -> Result<RepoRulesetState> {
    let has_copilot_review = if check_copilot_review {
        client
            .list_rulesets(repo)
            .await?
            .iter()
            .any(|ruleset| ruleset.name == "Copilot Code Review")
    } else {
        true
    };

    let has_instructions = if check_instructions {
        client
            .get_file(repo, ".github/copilot-instructions.md", None)
            .await?
            .is_some()
    } else {
        true
    };

    let repository_changes = if let Some(desired) = desired_repository {
        let (settings, topics) =
            tokio::try_join!(client.get_settings(repo), client.get_topics(repo))?;
        diff_repository_settings(&settings, &topics, desired)
    } else {
        Vec::new()
    };

    Ok(RepoRulesetState {
        repo: repo.to_owned(),
        has_copilot_review,
        has_instructions,
        is_ops: is_ops_repo(repo),
        repository_changes,
    })
}

fn diff_repository_settings(
    current: &RepoSettings,
    topics: &[String],
    desired: &RepositorySettingsConfig,
) -> Vec<RepositorySettingChange> {
    let mut changes = Vec::new();

    compare_value(
        &mut changes,
        "has_issues",
        current.has_issues,
        desired.has_issues,
    );
    compare_value(
        &mut changes,
        "has_projects",
        current.has_projects,
        desired.has_projects,
    );
    compare_value(&mut changes, "has_wiki", current.has_wiki, desired.has_wiki);
    compare_value(
        &mut changes,
        "has_discussions",
        current.has_discussions,
        desired.has_discussions,
    );
    compare_value(
        &mut changes,
        "allow_squash_merge",
        current.allow_squash_merge,
        desired.allow_squash_merge,
    );
    compare_value(
        &mut changes,
        "allow_merge_commit",
        current.allow_merge_commit,
        desired.allow_merge_commit,
    );
    compare_value(
        &mut changes,
        "allow_rebase_merge",
        current.allow_rebase_merge,
        desired.allow_rebase_merge,
    );
    compare_value(
        &mut changes,
        "allow_auto_merge",
        current.allow_auto_merge,
        desired.allow_auto_merge,
    );
    compare_value(
        &mut changes,
        "delete_branch_on_merge",
        current.delete_branch_on_merge,
        desired.delete_branch_on_merge,
    );
    compare_value(
        &mut changes,
        "allow_update_branch",
        current.allow_update_branch,
        desired.allow_update_branch,
    );
    compare_optional_string(
        &mut changes,
        "squash_merge_commit_title",
        current.squash_merge_commit_title.as_deref(),
        desired.squash_merge_commit_title.as_deref(),
    );
    compare_optional_string(
        &mut changes,
        "squash_merge_commit_message",
        current.squash_merge_commit_message.as_deref(),
        desired.squash_merge_commit_message.as_deref(),
    );
    compare_optional_string(
        &mut changes,
        "merge_commit_title",
        current.merge_commit_title.as_deref(),
        desired.merge_commit_title.as_deref(),
    );
    compare_optional_string(
        &mut changes,
        "merge_commit_message",
        current.merge_commit_message.as_deref(),
        desired.merge_commit_message.as_deref(),
    );
    compare_value(
        &mut changes,
        "web_commit_signoff_required",
        current.web_commit_signoff_required,
        desired.web_commit_signoff_required,
    );

    if let Some(desired_topics) = &desired.topics
        && topics != desired_topics
    {
        changes.push(RepositorySettingChange {
            field: "topics",
            current: format!("{topics:?}"),
            desired: format!("{desired_topics:?}"),
        });
    }

    changes
}

fn compare_value<T>(
    changes: &mut Vec<RepositorySettingChange>,
    field: &'static str,
    current: T,
    desired: Option<T>,
) where
    T: fmt::Display + PartialEq,
{
    if let Some(desired) = desired
        && current != desired
    {
        changes.push(RepositorySettingChange {
            field,
            current: current.to_string(),
            desired: desired.to_string(),
        });
    }
}

fn compare_optional_string(
    changes: &mut Vec<RepositorySettingChange>,
    field: &'static str,
    current: Option<&str>,
    desired: Option<&str>,
) {
    if let Some(desired) = desired
        && current != Some(desired)
    {
        changes.push(RepositorySettingChange {
            field,
            current: current.unwrap_or("<unset>").to_owned(),
            desired: desired.to_owned(),
        });
    }
}

fn repository_settings_patch(config: &RepositorySettingsConfig) -> serde_json::Value {
    let mut body = serde_json::Map::new();

    macro_rules! insert {
        ($field:ident) => {
            if let Some(value) = &config.$field {
                body.insert(stringify!($field).to_owned(), serde_json::json!(value));
            }
        };
    }

    insert!(has_issues);
    insert!(has_projects);
    insert!(has_wiki);
    insert!(has_discussions);
    insert!(allow_squash_merge);
    insert!(allow_merge_commit);
    insert!(allow_rebase_merge);
    insert!(allow_auto_merge);
    insert!(delete_branch_on_merge);
    insert!(allow_update_branch);
    insert!(squash_merge_commit_title);
    insert!(squash_merge_commit_message);
    insert!(merge_commit_title);
    insert!(merge_commit_message);
    insert!(web_commit_signoff_required);

    serde_json::Value::Object(body)
}

async fn apply_repository_settings(
    client: &Client,
    repo: &str,
    desired: &RepositorySettingsConfig,
) -> Result<()> {
    let patch = repository_settings_patch(desired);
    if patch.as_object().is_some_and(|body| !body.is_empty()) {
        client.update_settings(repo, &patch).await?;
    }
    if let Some(topics) = &desired.topics {
        client.replace_topics(repo, topics).await?;
    }

    let (current, topics) = tokio::try_join!(client.get_settings(repo), client.get_topics(repo))?;
    let remaining = diff_repository_settings(&current, &topics, desired);
    if !remaining.is_empty() {
        anyhow::bail!(
            "Repository settings verification failed for {repo}: {} field(s) still differ",
            remaining.len()
        );
    }

    Ok(())
}

pub(crate) async fn repository_settings_compliant(
    client: &Client,
    repo: &str,
    desired: &RepositorySettingsConfig,
) -> Result<bool> {
    let (current, topics) = tokio::try_join!(client.get_settings(repo), client.get_topics(repo))?;
    Ok(diff_repository_settings(&current, &topics, desired).is_empty())
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
        .list_repos_for_system(
            sys,
            manifest.matches_prefix_for_system(sys),
            &excludes,
            &explicit,
        )
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
    let do_ruleset = ruleset.is_some();
    let do_instructions = copilot_instructions;
    if manifest.repository.is_none() && !do_ruleset && !do_instructions {
        anyhow::bail!(
            "No [repository] settings configured. Use --ruleset or --copilot-instructions for Copilot setup."
        );
    }

    println!();
    println!(
        "  {} Settings plan: scanning {} repos...",
        style("[..]").bold(),
        repos.len()
    );
    println!();

    let mut ruleset_needed = 0;
    let mut instructions_needed = 0;
    let mut repository_settings_needed = 0;
    let mut up_to_date = 0;

    for repo_name in &repos {
        let state = scan_repo(
            client,
            repo_name,
            manifest.repository.as_ref(),
            do_ruleset,
            do_instructions,
        )
        .await?;
        let mut changes: Vec<String> = state
            .repository_changes
            .iter()
            .map(|change| {
                format!(
                    "set {}: {} -> {}",
                    change.field, change.current, change.desired
                )
            })
            .collect();
        if !state.repository_changes.is_empty() {
            repository_settings_needed += 1;
        }

        if do_ruleset && !state.has_copilot_review {
            changes.push("create Copilot Code Review ruleset".to_owned());
            ruleset_needed += 1;
        }

        if do_instructions && !state.has_instructions {
            changes.push(if state.is_ops {
                "deploy copilot-instructions.md (ops)".to_owned()
            } else {
                "deploy copilot-instructions.md (app)".to_owned()
            });
            instructions_needed += 1;
        }

        if changes.is_empty() {
            println!("  {} {}", style("[ok]").green(), style(repo_name).dim());
            up_to_date += 1;
        } else {
            println!("  {} {}", style("[>>]").yellow(), style(repo_name).bold());
            for change in &changes {
                println!("     {change}");
            }
        }
    }

    println!();
    println!(
        "  Summary: {} need repository settings, {} need ruleset, {} need instructions, {} up to date",
        style(repository_settings_needed).yellow().bold(),
        style(ruleset_needed).yellow().bold(),
        style(instructions_needed).yellow().bold(),
        style(up_to_date).green()
    );

    if repository_settings_needed + ruleset_needed + instructions_needed > 0 {
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
    let do_ruleset = ruleset.is_some();
    let do_instructions = copilot_instructions;
    if manifest.repository.is_none() && !do_ruleset && !do_instructions {
        anyhow::bail!(
            "No [repository] settings configured. Use --ruleset or --copilot-instructions for Copilot setup."
        );
    }
    let branch_name = &manifest.templates.branch;

    println!();
    println!(
        "  {} Scanning {} repos...",
        style("[..]").bold(),
        repos.len()
    );

    // Scan all repos
    let mut work: Vec<(RepoRulesetState, String)> = Vec::new();
    for repo_name in &repos {
        let state = scan_repo(
            client,
            repo_name,
            manifest.repository.as_ref(),
            do_ruleset,
            do_instructions,
        )
        .await?;
        let r = client.get_repo(repo_name).await?;
        let needs_work = !state.repository_changes.is_empty()
            || (do_ruleset && !state.has_copilot_review)
            || (do_instructions && !state.has_instructions);
        if needs_work {
            work.push((state, r.default_branch));
        }
    }

    if work.is_empty() {
        println!("\n  {} All repos up to date.", style("[ok]").green());
        return Ok(());
    }

    println!(
        "\n  {} repos need changes:",
        style(work.len()).yellow().bold()
    );
    for (state, _) in &work {
        let mut actions = Vec::new();
        if !state.repository_changes.is_empty() {
            actions.push("repository settings");
        }
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
            style("[>>]").yellow(),
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
        println!("  {} {} ...", style(">>").magenta(), state.repo);

        if !state.repository_changes.is_empty()
            && let Some(desired) = &manifest.repository
        {
            match apply_repository_settings(client, &state.repo, desired).await {
                Ok(()) => {
                    println!("    {} Repository settings updated", style("[ok]").green());
                    audit_log.log(
                        &state.repo,
                        "update_repository_settings",
                        "success",
                        false,
                        true,
                    )?;
                }
                Err(e) => {
                    println!("    {} Repository settings: {e}", style("[!!]").red());
                    failed.push((state.repo.clone(), format!("repository settings: {e}")));
                    continue;
                }
            }
        }

        // Create ruleset
        if do_ruleset && !state.has_copilot_review {
            match client.create_copilot_review_ruleset(&state.repo).await {
                Ok(()) => {
                    println!(
                        "    {} Copilot review ruleset created",
                        style("[ok]").green()
                    );
                    audit_log.log(
                        &state.repo,
                        "create_copilot_review_ruleset",
                        "success",
                        false,
                        true,
                    )?;
                }
                Err(e) => {
                    println!("    {} Ruleset: {e}", style("[!!]").red());
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
                                style("[ok]").green(),
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
                            println!("    {} Instructions: {e}", style("[!!]").red());
                            failed.push((state.repo.clone(), format!("instructions: {e}")));
                            continue;
                        }
                    }
                }
                Err(e) => {
                    println!("    {} Template render: {e}", style("[!!]").red());
                    failed.push((state.repo.clone(), format!("template: {e}")));
                    continue;
                }
            }
        }

        succeeded += 1;
    }

    println!();
    if failed.is_empty() {
        println!(
            "  {} All {} repos updated.",
            style("[ok]").green(),
            succeeded
        );
    } else {
        println!(
            "  {} {} succeeded, {} failed:",
            style("[warn]").yellow(),
            succeeded,
            failed.len()
        );
        for (repo, err) in &failed {
            println!("    {} {}: {}", style("[!!]").red(), repo, err);
        }
    }

    println!(
        "\n  {} Audit log: {}",
        style("[..]").bold(),
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
        style("[..]").bold(),
        repos.len()
    );

    use tabled::builder::Builder;
    use tabled::settings::object::{Columns, Rows};
    use tabled::settings::{Alignment, Modify, Style};

    let mut builder = Builder::default();
    builder.push_record([
        "Repository",
        "Type",
        "Repo Settings",
        "Review Rule",
        "Instructions",
    ]);

    let mut all_ok = 0;
    let mut issues = 0;

    for repo_name in &repos {
        let state = scan_repo(client, repo_name, manifest.repository.as_ref(), true, true).await?;

        let repository_icon = if state.repository_changes.is_empty() {
            format!("{}", style("[ok]").green())
        } else {
            format!("{}", style("[!!]").red())
        };
        let ruleset_icon = if state.has_copilot_review {
            format!("{}", style("[ok]").green())
        } else {
            format!("{}", style("[!!]").red())
        };
        let instr_icon = if state.has_instructions {
            format!("{}", style("[ok]").green())
        } else {
            format!("{}", style("[!!]").red())
        };
        let repo_type = if state.is_ops { "ops" } else { "app" };

        let ok = state.repository_changes.is_empty()
            && state.has_copilot_review
            && state.has_instructions;
        if ok {
            all_ok += 1;
        } else {
            issues += 1;
        }

        builder.push_record([
            repo_name.as_str(),
            repo_type,
            &repository_icon,
            &ruleset_icon,
            &instr_icon,
        ]);
    }

    let table = builder
        .build()
        .with(Style::blank())
        .with(
            Modify::new(Rows::first()).with(tabled::settings::Format::content(|s| {
                format!("{}", style(s).bold().underlined())
            })),
        )
        .with(Modify::new(Columns::new(..)).with(Alignment::left()))
        .to_string();

    println!();
    for line in table.lines() {
        println!("  {line}");
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

    fn repository_settings() -> RepoSettings {
        RepoSettings {
            has_issues: true,
            has_projects: false,
            has_wiki: false,
            has_discussions: true,
            has_pull_requests: true,
            pull_request_creation_policy: Some("all".to_owned()),
            allow_squash_merge: true,
            allow_merge_commit: false,
            allow_rebase_merge: true,
            allow_auto_merge: true,
            delete_branch_on_merge: true,
            allow_update_branch: true,
            squash_merge_commit_title: Some("PR_TITLE".to_owned()),
            squash_merge_commit_message: Some("PR_BODY".to_owned()),
            merge_commit_title: Some("PR_TITLE".to_owned()),
            merge_commit_message: Some("PR_BODY".to_owned()),
            web_commit_signoff_required: true,
            use_squash_pr_title_as_default: Some(false),
        }
    }

    #[test]
    fn repository_settings_diff_is_empty_for_matching_snapshot() {
        let desired = RepositorySettingsConfig {
            has_issues: Some(true),
            has_projects: Some(false),
            has_wiki: Some(false),
            has_discussions: Some(true),
            has_pull_requests: None,
            pull_request_creation_policy: None,
            has_sponsorships_enabled: None,
            issue_creation_policy: None,
            allow_squash_merge: Some(true),
            allow_merge_commit: Some(false),
            allow_rebase_merge: Some(true),
            allow_auto_merge: Some(true),
            delete_branch_on_merge: Some(true),
            allow_update_branch: Some(true),
            squash_merge_commit_title: Some("PR_TITLE".to_owned()),
            squash_merge_commit_message: Some("PR_BODY".to_owned()),
            merge_commit_title: Some("PR_TITLE".to_owned()),
            merge_commit_message: Some("PR_BODY".to_owned()),
            web_commit_signoff_required: Some(true),
            use_squash_pr_title_as_default: None,
            topics: Some(vec!["managed".to_owned()]),
        };

        let changes =
            diff_repository_settings(&repository_settings(), &["managed".to_owned()], &desired);
        assert!(changes.is_empty());
    }

    #[test]
    fn repository_settings_diff_reports_only_configured_drift() {
        let desired = RepositorySettingsConfig {
            allow_auto_merge: Some(false),
            topics: Some(vec!["baseline".to_owned()]),
            ..RepositorySettingsConfig::default()
        };

        let changes =
            diff_repository_settings(&repository_settings(), &["managed".to_owned()], &desired);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].field, "allow_auto_merge");
        assert_eq!(changes[1].field, "topics");
    }

    #[test]
    fn repository_settings_patch_excludes_topics() {
        let desired = RepositorySettingsConfig {
            has_issues: Some(false),
            topics: Some(vec!["baseline".to_owned()]),
            ..RepositorySettingsConfig::default()
        };

        let patch = repository_settings_patch(&desired);
        assert_eq!(patch["has_issues"], false);
        assert!(patch.get("topics").is_none());
    }

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
