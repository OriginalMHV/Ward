use anyhow::Result;
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::config::manifest::{
    ManagementDisposition, RepositoryCategoryV2, RepositorySettingsConfig,
};
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::reconcile::general::{
    self, GeneralChange, GeneralChangeKind, GeneralDesiredState, GeneralPlan,
};
use crate::reconcile::unified;

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
    },

    /// Apply settings and rulesets
    Apply {
        /// Ruleset to apply (copilot-review)
        #[arg(long)]
        ruleset: Option<String>,

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
            SettingsAction::Plan { ruleset } => {
                plan(client, manifest, system, repo, ruleset.as_deref()).await
            }
            SettingsAction::Apply { ruleset, yes } => {
                apply(client, manifest, system, repo, ruleset.as_deref(), *yes).await
            }
            SettingsAction::Audit => audit(client, manifest, system, repo).await,
        }
    }
}

fn repository_category_for_repo<'a>(
    manifest: &'a Manifest,
    repo_name: &str,
) -> Option<&'a RepositoryCategoryV2> {
    let system_repository = manifest
        .system_for_repo(repo_name)
        .and_then(|system_id| manifest.system(system_id))
        .and_then(|system| system.categories.repository.as_ref());
    system_repository.or(manifest.categories.repository.as_ref())
}

fn managed_repository_settings_for_repo<'a>(
    manifest: &'a Manifest,
    repo_name: &str,
) -> Option<&'a RepositorySettingsConfig> {
    repository_category_for_repo(manifest, repo_name)
        .filter(|repository| repository.policy.disposition == ManagementDisposition::Managed)
        .and_then(|repository| repository.settings.as_ref())
}

fn settings_desired_for_repo(
    manifest: &Manifest,
    repo_name: &str,
    managed_only: bool,
) -> Option<GeneralDesiredState> {
    let category = repository_category_for_repo(manifest, repo_name)?;
    if managed_only && category.policy.disposition != ManagementDisposition::Managed {
        return None;
    }
    let settings = category.settings.as_ref()?;
    Some(settings_desired_state(category, settings))
}

struct RepoRulesetState {
    repo: String,
    has_copilot_review: bool,
    repository_changes: Vec<RepositorySettingChange>,
    plan: Option<GeneralPlan>,
}

#[derive(Debug)]
struct RepositorySettingChange {
    field: String,
    current: String,
    desired: String,
}

impl From<&GeneralChange> for RepositorySettingChange {
    fn from(change: &GeneralChange) -> Self {
        let field = match &change.kind {
            GeneralChangeKind::RestField { field } | GeneralChangeKind::GraphqlField { field } => {
                field.clone()
            }
            GeneralChangeKind::Topics => "topics".to_owned(),
            other => format!("{other:?}"),
        };
        Self {
            field,
            current: change.current.clone(),
            desired: change.desired.clone(),
        }
    }
}

/// Desired state for the legacy settings command: only `settings` and topics.
/// Pruning is off so labels and custom properties are never touched.
fn settings_desired_state(
    category: &RepositoryCategoryV2,
    settings: &RepositorySettingsConfig,
) -> GeneralDesiredState {
    let mut repository = RepositoryCategoryV2 {
        policy: category.policy.clone(),
        settings: Some(settings.clone()),
        metadata: None,
        custom_properties: Vec::new(),
        immutable_releases: None,
        references: Vec::new(),
    };
    repository.policy.prune = false;
    GeneralDesiredState::from(repository)
}

async fn scan_repo(
    client: &Client,
    repo: &str,
    desired_repository: Option<GeneralDesiredState>,
    check_copilot_review: bool,
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

    let (repository_changes, plan) = if let Some(desired) = desired_repository {
        let current = general::collect(client, repo).await?;
        let plan = general::plan(repo, &desired, &current);
        let changes = plan
            .changes
            .iter()
            .chain(plan.blocked_changes.iter())
            .map(RepositorySettingChange::from)
            .collect();
        (changes, Some(plan))
    } else {
        (Vec::new(), None)
    };

    Ok(RepoRulesetState {
        repo: repo.to_owned(),
        has_copilot_review,
        repository_changes,
        plan,
    })
}

async fn scan_repos(
    client: &Client,
    manifest: &Manifest,
    repos: &[String],
    managed_only: bool,
    check_copilot_review: bool,
) -> Vec<Result<RepoRulesetState>> {
    crate::reconcile::map_buffered(repos, |repo_name| async move {
        let desired = settings_desired_for_repo(manifest, repo_name, managed_only);
        scan_repo(client, repo_name, desired, check_copilot_review).await
    })
    .await
}

async fn resolve_repos(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<String>> {
    if system.is_none() && repo.is_none() {
        anyhow::bail!("Either --system or --repo is required");
    }
    let repos = unified::resolve_target_repos(client, manifest, system, repo).await?;
    Ok(repos.into_iter().map(|r| r.name).collect())
}

async fn plan(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    ruleset: Option<&str>,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;
    let do_ruleset = ruleset.is_some();
    if !do_ruleset
        && !repos
            .iter()
            .any(|repo_name| managed_repository_settings_for_repo(manifest, repo_name).is_some())
    {
        anyhow::bail!(
            "No managed [categories.repository.settings] configured. Use --ruleset for Copilot review setup."
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
    let mut repository_settings_needed = 0;
    let mut up_to_date = 0;

    let states = scan_repos(client, manifest, &repos, true, do_ruleset).await;

    for (repo_name, state) in repos.iter().zip(states) {
        let state = state?;
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
        "  Summary: {} need repository settings, {} need ruleset, {} up to date",
        style(repository_settings_needed).yellow().bold(),
        style(ruleset_needed).yellow().bold(),
        style(up_to_date).green()
    );

    if repository_settings_needed + ruleset_needed > 0 {
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
    yes: bool,
) -> Result<()> {
    let repos = resolve_repos(client, manifest, system, repo).await?;
    let do_ruleset = ruleset.is_some();
    if !do_ruleset
        && !repos
            .iter()
            .any(|repo_name| managed_repository_settings_for_repo(manifest, repo_name).is_some())
    {
        anyhow::bail!(
            "No managed [categories.repository.settings] configured. Use --ruleset for Copilot review setup."
        );
    }

    println!();
    println!(
        "  {} Scanning {} repos...",
        style("[..]").bold(),
        repos.len()
    );

    // Scan all repos
    let mut work: Vec<RepoRulesetState> = Vec::new();
    for state in scan_repos(client, manifest, &repos, true, do_ruleset).await {
        let state = state?;
        let needs_work =
            !state.repository_changes.is_empty() || (do_ruleset && !state.has_copilot_review);
        if needs_work {
            work.push(state);
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
    for state in &work {
        let mut actions = Vec::new();
        if !state.repository_changes.is_empty() {
            actions.push("repository settings");
        }
        if do_ruleset && !state.has_copilot_review {
            actions.push("ruleset");
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
    let mut succeeded = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    for state in &work {
        println!("  {} {} ...", style(">>").magenta(), state.repo);

        if !state.repository_changes.is_empty()
            && let Some(plan) = state.plan.as_ref()
        {
            match general::apply(client, plan).await {
                Ok(_) => {
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
    builder.push_record(["Repository", "Repo Settings", "Review Rule"]);

    let mut all_ok = 0;
    let mut issues = 0;

    let states = scan_repos(client, manifest, &repos, false, true).await;

    for (repo_name, state) in repos.iter().zip(states) {
        let state = state?;

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

        let ok = state.repository_changes.is_empty() && state.has_copilot_review;
        if ok {
            all_ok += 1;
        } else {
            issues += 1;
        }

        builder.push_record([repo_name.as_str(), &repository_icon, &ruleset_icon]);
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
    use crate::config::manifest::CategoryPolicy;

    fn managed_category(settings: RepositorySettingsConfig) -> RepositoryCategoryV2 {
        RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: Some(settings),
            metadata: None,
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        }
    }

    fn collected(topics: &[&str]) -> general::CollectedGeneralState {
        let mut state = general::CollectedGeneralState {
            repository: managed_category(RepositorySettingsConfig {
                has_issues: Some(true),
                use_squash_pr_title_as_default: None,
                topics: Some(topics.iter().map(|topic| (*topic).to_owned()).collect()),
                ..RepositorySettingsConfig::default()
            }),
            labels: Vec::new(),
            custom_properties: Vec::new(),
            coverage: Vec::new(),
            extensions: general::GeneralCollectedExtensions::default(),
        };
        state.extensions.graphql_settings_collected = true;
        state.extensions.has_pull_requests = Some(true);
        state.extensions.pull_request_creation_policy = Some("all".to_owned());
        state.extensions.has_sponsorships_enabled = Some(false);
        state.extensions.issue_creation_policy = Some("all".to_owned());
        state.extensions.use_squash_pr_title_as_default = Some(false);
        state
    }

    fn changed_fields(
        settings: RepositorySettingsConfig,
        current: &general::CollectedGeneralState,
    ) -> Vec<String> {
        let category = managed_category(settings.clone());
        let desired = settings_desired_state(&category, &settings);
        general::plan("repo", &desired, current)
            .changes
            .iter()
            .map(|change| RepositorySettingChange::from(change).field)
            .collect()
    }

    #[test]
    fn settings_plan_covers_pull_request_sponsorship_and_policy_fields() {
        let fields = changed_fields(
            RepositorySettingsConfig {
                has_pull_requests: Some(false),
                pull_request_creation_policy: Some("collaborators_only".to_owned()),
                has_sponsorships_enabled: Some(true),
                issue_creation_policy: Some("collaborators_only".to_owned()),
                use_squash_pr_title_as_default: Some(true),
                ..RepositorySettingsConfig::default()
            },
            &collected(&[]),
        );
        for expected in [
            "has_pull_requests",
            "pull_request_creation_policy",
            "has_sponsorships_enabled",
            "issue_creation_policy",
            "use_squash_pr_title_as_default",
        ] {
            assert!(
                fields.iter().any(|field| field == expected),
                "{expected} missing from {fields:?}"
            );
        }
    }

    #[test]
    fn settings_plan_ignores_topic_order_and_case() {
        let fields = changed_fields(
            RepositorySettingsConfig {
                topics: Some(vec!["Beta".to_owned(), "alpha".to_owned()]),
                ..RepositorySettingsConfig::default()
            },
            &collected(&["alpha", "beta"]),
        );
        assert!(fields.is_empty(), "{fields:?}");
    }

    #[test]
    fn settings_desired_state_never_prunes() {
        let mut category = managed_category(RepositorySettingsConfig::default());
        category.policy.prune = true;
        let desired = settings_desired_state(&category, &RepositorySettingsConfig::default());
        assert!(!desired.repository.policy.prune);
    }
}
