use std::collections::HashMap;

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use dialoguer::{Confirm, Input, MultiSelect};
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::Deserialize;

use crate::config::auth;

const EXAMPLE_MANIFEST: &str = r#"[org]
name = "your-github-org"

[security]
secret_scanning = true
secret_scanning_ai_detection = true
push_protection = true
dependabot_alerts = true
dependabot_security_updates = true

[templates]
branch = "chore/ward-setup"
reviewers = []
commit_message_prefix = "chore: "

# [[systems]]
# id = "my-system"
# name = "My System"
# exclude = ["operations?", "workflows"]
"#;

const OUTPUT_PATH: &str = "ward.toml";

#[derive(Args)]
pub struct InitCommand {
    /// Skip interactive wizard, write default ward.toml
    #[arg(long)]
    non_interactive: bool,
}

impl InitCommand {
    pub async fn run(&self) -> Result<()> {
        if self.non_interactive {
            return write_default();
        }
        run_wizard().await
    }
}

fn write_default() -> Result<()> {
    if std::path::Path::new(OUTPUT_PATH).exists() {
        println!("  {} ward.toml already exists.", style("warning").yellow());
        return Ok(());
    }

    std::fs::write(OUTPUT_PATH, EXAMPLE_MANIFEST)?;
    println!(
        "  {} Created ward.toml - edit it to configure your org and systems.",
        style("ok").green()
    );
    Ok(())
}

// --- Wizard ---

struct WizardState {
    org: String,
    repo_count: usize,
    security: SecuritySettings,
    branch_protection: BranchProtectionSettings,
    systems: Vec<SystemEntry>,
    exclude_patterns: Vec<String>,
    templates: TemplateSettings,
}

struct SecuritySettings {
    secret_scanning: bool,
    push_protection: bool,
    dependabot_alerts: bool,
    dependabot_security_updates: bool,
}

struct BranchProtectionSettings {
    enabled: bool,
    required_approvals: u32,
    dismiss_stale_reviews: bool,
}

struct SystemEntry {
    id: String,
    name: String,
    repo_count: usize,
}

struct TemplateSettings {
    branch: String,
    reviewers: Vec<String>,
    commit_message_prefix: String,
}

fn print_banner() {
    println!();
    println!("  {}", style("+---------------------------------+").cyan());
    println!("  {}", style("|       Ward Setup Wizard         |").cyan());
    println!("  {}", style("+---------------------------------+").cyan());
    println!();
}

fn print_step(step: u8, total: u8, title: &str) {
    println!();
    println!(
        "  {} {}",
        style(format!("Step {step}/{total}:")).bold(),
        style(title).bold()
    );
}

async fn run_wizard() -> Result<()> {
    if std::path::Path::new(OUTPUT_PATH).exists() {
        println!("  {} ward.toml already exists.", style("warning").yellow());
        return Ok(());
    }

    print_banner();

    let total_steps = 6;

    // Step 1: Auth
    print_step(1, total_steps, "Authentication");
    let token = check_auth()?;

    // Step 2: Org
    print_step(2, total_steps, "Organization");
    let (org, repo_count) = ask_org(&token).await?;

    // Step 3: Security
    print_step(3, total_steps, "Security Settings");
    let security = ask_security()?;

    // Step 4: Branch protection
    print_step(4, total_steps, "Branch Protection");
    let branch_protection = ask_branch_protection()?;

    // Step 5: Systems
    print_step(5, total_steps, "Systems");
    let (systems, exclude_patterns) = discover_systems(&token, &org).await?;

    // Step 6: Templates
    print_step(6, total_steps, "Templates");
    let templates = ask_templates()?;

    let state = WizardState {
        org,
        repo_count,
        security,
        branch_protection,
        systems,
        exclude_patterns,
        templates,
    };

    write_toml(&state)?;
    print_summary(&state);

    Ok(())
}

// Step 1: Check authentication

fn check_auth() -> Result<String> {
    match auth::resolve_token() {
        Ok(token) => {
            let source = if std::env::var("GH_TOKEN").is_ok() {
                "GH_TOKEN"
            } else if std::env::var("GITHUB_TOKEN").is_ok() {
                "GITHUB_TOKEN"
            } else {
                "gh auth token"
            };
            println!("  {} Token found via {source}", style("ok").green());
            Ok(token)
        }
        Err(e) => {
            println!("  {} No GitHub token found.", style("error").red());
            println!("  Set GH_TOKEN, GITHUB_TOKEN, or run `gh auth login`.");
            Err(e)
        }
    }
}

// Step 2: Ask for org and verify

async fn verify_org(token: &str, org: &str) -> Result<usize> {
    let client = build_http_client(token)?;
    let resp = client
        .get(format!("https://api.github.com/orgs/{org}"))
        .send()
        .await
        .context("Failed to reach GitHub API")?;

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("Organization '{org}' not found");
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Failed to verify org (HTTP {status}): {body}");
    }

    // Org exists - consume the response body, then count repos via pagination
    let _ = resp.bytes().await;
    let repos = fetch_all_repos(token, org).await?;
    Ok(repos.len())
}

async fn ask_org(token: &str) -> Result<(String, usize)> {
    loop {
        let org: String = Input::new()
            .with_prompt("  GitHub organization name")
            .interact_text()?;

        match verify_org(token, &org).await {
            Ok(count) => {
                println!(
                    "  {} Organization verified ({count} repos)",
                    style("ok").green()
                );
                return Ok((org, count));
            }
            Err(e) => {
                println!("  {} {e}", style("error").red());
                println!("  Please try again.");
            }
        }
    }
}

// Step 3: Security settings

fn ask_security() -> Result<SecuritySettings> {
    let secret_scanning = Confirm::new()
        .with_prompt("  Enable secret scanning?")
        .default(true)
        .interact()?;

    let push_protection = Confirm::new()
        .with_prompt("  Enable push protection?")
        .default(true)
        .interact()?;

    let dependabot_alerts = Confirm::new()
        .with_prompt("  Enable Dependabot alerts?")
        .default(true)
        .interact()?;

    let dependabot_security_updates = Confirm::new()
        .with_prompt("  Enable Dependabot security updates?")
        .default(true)
        .interact()?;

    Ok(SecuritySettings {
        secret_scanning,
        push_protection,
        dependabot_alerts,
        dependabot_security_updates,
    })
}

// Step 4: Branch protection

fn ask_branch_protection() -> Result<BranchProtectionSettings> {
    let enabled = Confirm::new()
        .with_prompt("  Enable branch protection?")
        .default(true)
        .interact()?;

    if !enabled {
        return Ok(BranchProtectionSettings {
            enabled: false,
            required_approvals: 1,
            dismiss_stale_reviews: false,
        });
    }

    let approvals: String = Input::new()
        .with_prompt("  Required approvals")
        .default("1".to_owned())
        .interact_text()?;
    let required_approvals: u32 = approvals.parse().unwrap_or(1);

    let dismiss_stale_reviews = Confirm::new()
        .with_prompt("  Dismiss stale reviews?")
        .default(true)
        .interact()?;

    Ok(BranchProtectionSettings {
        enabled,
        required_approvals,
        dismiss_stale_reviews,
    })
}

// Step 5: Discover systems

#[derive(Debug, Clone)]
struct DiscoveredPrefix {
    prefix: String,
    count: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct MinimalRepo {
    name: String,
    archived: bool,
}

async fn fetch_all_repos(token: &str, org: &str) -> Result<Vec<MinimalRepo>> {
    let client = build_http_client(token)?;
    let mut all = Vec::new();
    let mut page = 1u32;

    loop {
        let resp = client
            .get(format!(
                "https://api.github.com/orgs/{org}/repos?per_page=100&page={page}&type=all"
            ))
            .send()
            .await
            .context("Failed to fetch repos")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to list repos (HTTP {status}): {body}");
        }

        let repos: Vec<MinimalRepo> = resp.json().await.context("Failed to parse repos")?;
        if repos.is_empty() {
            break;
        }

        all.extend(repos);
        page += 1;
    }

    Ok(all)
}

fn discover_prefixes(repos: &[MinimalRepo]) -> Vec<DiscoveredPrefix> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    for repo in repos {
        if repo.archived {
            continue;
        }
        if let Some(prefix) = repo.name.split('-').next()
            && !prefix.is_empty()
        {
            *counts.entry(prefix.to_owned()).or_default() += 1;
        }
    }

    let mut prefixes: Vec<DiscoveredPrefix> = counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(prefix, count)| DiscoveredPrefix { prefix, count })
        .collect();

    prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.count));
    prefixes
}

async fn discover_systems(token: &str, org: &str) -> Result<(Vec<SystemEntry>, Vec<String>)> {
    println!("  Scanning repos...");
    let repos = fetch_all_repos(token, org).await?;
    let active_count = repos.iter().filter(|r| !r.archived).count();
    println!(
        "  Found {active_count} active repos (of {} total)",
        repos.len()
    );

    let prefixes = discover_prefixes(&repos);

    if prefixes.is_empty() {
        println!(
            "  {} No common prefixes found. You can add systems manually to ward.toml.",
            style("info").blue()
        );
        let exclude = ask_exclude_patterns()?;
        return Ok((Vec::new(), exclude));
    }

    println!();
    println!("  Discovered prefixes:");
    for p in &prefixes {
        println!("    {} - {} repos", style(&p.prefix).cyan(), p.count);
    }
    println!();

    let items: Vec<String> = prefixes
        .iter()
        .map(|p| format!("{} ({} repos)", p.prefix, p.count))
        .collect();

    let selected = MultiSelect::new()
        .with_prompt("  Select systems to manage")
        .items(&items)
        .defaults(&vec![true; items.len()])
        .interact()?;

    let mut systems = Vec::new();
    for idx in selected {
        let p = &prefixes[idx];
        let name: String = Input::new()
            .with_prompt(format!("  Name for system {}", style(&p.prefix).cyan()))
            .default(p.prefix.clone())
            .interact_text()?;

        systems.push(SystemEntry {
            id: p.prefix.clone(),
            name,
            repo_count: p.count,
        });
    }

    let exclude = ask_exclude_patterns()?;

    Ok((systems, exclude))
}

fn ask_exclude_patterns() -> Result<Vec<String>> {
    let raw: String = Input::new()
        .with_prompt("  Exclude patterns (comma-separated, regex)")
        .default("operations?,workflows".to_owned())
        .interact_text()?;

    let patterns: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(patterns)
}

// Step 6: Templates

fn ask_templates() -> Result<TemplateSettings> {
    let branch: String = Input::new()
        .with_prompt("  Branch name for PRs")
        .default("chore/ward-setup".to_owned())
        .interact_text()?;

    let reviewers_raw: String = Input::new()
        .with_prompt("  Reviewers (comma-separated)")
        .default(String::new())
        .allow_empty(true)
        .interact_text()?;

    let reviewers: Vec<String> = reviewers_raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();

    let commit_message_prefix: String = Input::new()
        .with_prompt("  Commit message prefix")
        .default("chore: ".to_owned())
        .interact_text()?;

    Ok(TemplateSettings {
        branch,
        reviewers,
        commit_message_prefix,
    })
}

// TOML generation

fn write_toml(state: &WizardState) -> Result<()> {
    let mut out = String::new();

    out.push_str("[org]\n");
    out.push_str(&format!("name = {:?}\n", state.org));

    out.push_str("\n[security]\n");
    out.push_str(&format!(
        "secret_scanning = {}\n",
        state.security.secret_scanning
    ));
    out.push_str(&format!(
        "push_protection = {}\n",
        state.security.push_protection
    ));
    out.push_str(&format!(
        "dependabot_alerts = {}\n",
        state.security.dependabot_alerts
    ));
    out.push_str(&format!(
        "dependabot_security_updates = {}\n",
        state.security.dependabot_security_updates
    ));

    out.push_str("\n[branch_protection]\n");
    out.push_str(&format!("enabled = {}\n", state.branch_protection.enabled));
    if state.branch_protection.enabled {
        out.push_str(&format!(
            "required_approvals = {}\n",
            state.branch_protection.required_approvals
        ));
        out.push_str(&format!(
            "dismiss_stale_reviews = {}\n",
            state.branch_protection.dismiss_stale_reviews
        ));
    }

    out.push_str("\n[templates]\n");
    out.push_str(&format!("branch = {:?}\n", state.templates.branch));
    let reviewers_toml: Vec<String> = state
        .templates
        .reviewers
        .iter()
        .map(|r| format!("{r:?}"))
        .collect();
    out.push_str(&format!("reviewers = [{}]\n", reviewers_toml.join(", ")));
    out.push_str(&format!(
        "commit_message_prefix = {:?}\n",
        state.templates.commit_message_prefix
    ));

    for sys in &state.systems {
        out.push('\n');
        out.push_str("[[systems]]\n");
        out.push_str(&format!("id = {:?}\n", sys.id));
        out.push_str(&format!("name = {:?}\n", sys.name));
        if !state.exclude_patterns.is_empty() {
            let exclude_toml: Vec<String> = state
                .exclude_patterns
                .iter()
                .map(|p| format!("{p:?}"))
                .collect();
            out.push_str(&format!("exclude = [{}]\n", exclude_toml.join(", ")));
        }
    }

    std::fs::write(OUTPUT_PATH, &out).context("Failed to write ward.toml")?;
    Ok(())
}

fn print_summary(state: &WizardState) {
    println!();
    println!(
        "  {} Created ward.toml with {} system(s) for {} ({} repos)",
        style("ok").green(),
        state.systems.len(),
        style(&state.org).cyan(),
        state.repo_count,
    );
    for sys in &state.systems {
        println!(
            "    - {} ({}) - {} repos",
            style(&sys.id).cyan(),
            sys.name,
            sys.repo_count,
        );
    }
    println!();
    println!("  Next steps:");
    println!("    ward repos list              - see matched repos");
    println!("    ward security plan            - preview security changes");
    println!("    ward security apply           - apply changes");
    println!();
}

// HTTP helpers

fn build_http_client(token: &str) -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).context("Invalid token characters")?,
    );
    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_static("ward-cli/0.1.0"),
    );

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("Failed to build HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_repo(name: &str, archived: bool) -> MinimalRepo {
        MinimalRepo {
            name: name.to_owned(),
            archived,
        }
    }

    #[test]
    fn discover_prefixes_groups_by_first_segment() {
        let repos = vec![
            make_repo("backend-foo", false),
            make_repo("backend-bar", false),
            make_repo("backend-baz", false),
            make_repo("frontend-one", false),
            make_repo("frontend-two", false),
        ];

        let prefixes = discover_prefixes(&repos);
        assert_eq!(prefixes.len(), 2);
        assert_eq!(prefixes[0].prefix, "backend");
        assert_eq!(prefixes[0].count, 3);
        assert_eq!(prefixes[1].prefix, "frontend");
        assert_eq!(prefixes[1].count, 2);
    }

    #[test]
    fn discover_prefixes_ignores_archived() {
        let repos = vec![
            make_repo("backend-foo", false),
            make_repo("backend-bar", true),
            make_repo("backend-baz", true),
        ];

        let prefixes = discover_prefixes(&repos);
        assert!(prefixes.is_empty());
    }

    #[test]
    fn discover_prefixes_filters_singletons() {
        let repos = vec![
            make_repo("backend-foo", false),
            make_repo("frontend-one", false),
            make_repo("frontend-two", false),
        ];

        let prefixes = discover_prefixes(&repos);
        assert_eq!(prefixes.len(), 1);
        assert_eq!(prefixes[0].prefix, "frontend");
    }

    #[test]
    fn discover_prefixes_sorts_by_count() {
        let repos = vec![
            make_repo("alpha-a", false),
            make_repo("alpha-b", false),
            make_repo("beta-a", false),
            make_repo("beta-b", false),
            make_repo("beta-c", false),
            make_repo("gamma-a", false),
            make_repo("gamma-b", false),
            make_repo("gamma-c", false),
            make_repo("gamma-d", false),
        ];

        let prefixes = discover_prefixes(&repos);
        assert_eq!(prefixes.len(), 3);
        assert_eq!(prefixes[0].prefix, "gamma");
        assert_eq!(prefixes[0].count, 4);
        assert_eq!(prefixes[1].prefix, "beta");
        assert_eq!(prefixes[1].count, 3);
        assert_eq!(prefixes[2].prefix, "alpha");
        assert_eq!(prefixes[2].count, 2);
    }

    #[test]
    fn discover_prefixes_handles_no_dash_names() {
        let repos = vec![make_repo("standalone", false), make_repo("another", false)];

        let prefixes = discover_prefixes(&repos);
        assert!(prefixes.is_empty());
    }

    #[test]
    fn discover_prefixes_empty_input() {
        let prefixes = discover_prefixes(&[]);
        assert!(prefixes.is_empty());
    }
}
