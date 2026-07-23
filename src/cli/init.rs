use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use dialoguer::{Confirm, Input, MultiSelect};
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::Deserialize;

use crate::cli::import::{ImportOptions, import_repository};
use crate::config::auth;

const EXAMPLE_MANIFEST: &str = r#"[org]
name = "your-github-org"

[schema]
version = 2

[file_delivery]
branch = "chore/ward-sync"
reviewers = []
commit_message_prefix = "chore: "

[categories.security]
secret_scanning = true
secret_scanning_push_protection = true
secret_scanning_ai_detection = true
dependabot_alerts = true
dependabot_security_updates = true

[categories.security.policy]
disposition = "managed"
prune = false
sensitive = true

# [[systems]]
# id = "my-system"
# name = "My System"
# match_prefix = true
# exclude = ["operations?", "workflows"]
"#;

const OUTPUT_PATH: &str = "ward.toml";

#[derive(Args)]
pub struct InitCommand {
    /// Build ward.toml from an existing repository (OWNER/REPO or GitHub URL)
    #[arg(long, conflicts_with = "non_interactive")]
    from: Option<String>,

    /// Skip interactive wizard, write default ward.toml
    #[arg(long)]
    non_interactive: bool,

    /// Output path for --from
    #[arg(long, default_value = OUTPUT_PATH, requires = "from")]
    output: PathBuf,

    /// Print the generated configuration for --from
    #[arg(long, requires = "from")]
    stdout: bool,

    /// Replace an existing output file for --from
    #[arg(long, requires = "from")]
    force: bool,

    /// Max concurrent API calls for --from
    #[arg(long, default_value_t = 5, requires = "from")]
    parallelism: usize,

    /// Existing target repository for --from. Repeat for multiple targets.
    #[arg(long, value_name = "OWNER/REPO", requires = "from")]
    target: Vec<String>,

    /// Include configuration files matching this glob. Repeatable.
    #[arg(long, value_name = "GLOB", requires = "from")]
    include: Vec<String>,

    /// Exclude configuration files matching this glob. Repeatable.
    #[arg(long, value_name = "GLOB", requires = "from")]
    exclude: Vec<String>,

    /// Fail if any readable source setting is unavailable.
    #[arg(long, requires = "from")]
    strict: bool,
}

impl InitCommand {
    pub async fn run(&self) -> Result<()> {
        if let Some(source) = &self.from {
            return import_repository(ImportOptions {
                source,
                targets: &self.target,
                include: &self.include,
                exclude: &self.exclude,
                strict: self.strict,
                output: &self.output,
                stdout: self.stdout,
                force: self.force,
                parallelism: self.parallelism,
            })
            .await;
        }

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
    file_delivery: FileDeliverySettings,
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

struct FileDeliverySettings {
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

    // Step 6: File delivery
    print_step(6, total_steps, "File Delivery");
    let file_delivery = ask_file_delivery()?;

    let state = WizardState {
        org,
        repo_count,
        security,
        branch_protection,
        systems,
        exclude_patterns,
        file_delivery,
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

// Step 6: File delivery

fn ask_file_delivery() -> Result<FileDeliverySettings> {
    let branch: String = Input::new()
        .with_prompt("  Branch name for PRs")
        .default("chore/ward-sync".to_owned())
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

    Ok(FileDeliverySettings {
        branch,
        reviewers,
        commit_message_prefix,
    })
}

// TOML generation

fn write_toml(state: &WizardState) -> Result<()> {
    std::fs::write(OUTPUT_PATH, render_manifest(state)).context("Failed to write ward.toml")?;
    Ok(())
}

fn render_manifest(state: &WizardState) -> String {
    let mut out = String::new();

    out.push_str("[org]\n");
    out.push_str(&format!("name = {:?}\n", state.org));

    out.push_str("\n[schema]\nversion = 2\n");

    out.push_str("\n[categories.security]\n");
    out.push_str(&format!(
        "secret_scanning = {}\n",
        state.security.secret_scanning
    ));
    out.push_str(&format!(
        "secret_scanning_push_protection = {}\n",
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
    out.push_str("\n[categories.security.policy]\n");
    out.push_str("disposition = \"managed\"\nprune = false\nsensitive = true\n");

    out.push_str("\n[categories.branch_protection.default_branch]\n");
    out.push_str(&format!("enabled = {}\n", state.branch_protection.enabled));
    out.push_str(&format!(
        "required_approvals = {}\n",
        state.branch_protection.required_approvals
    ));
    out.push_str(&format!(
        "dismiss_stale_reviews = {}\n",
        state.branch_protection.dismiss_stale_reviews
    ));
    out.push_str("\n[categories.branch_protection.policy]\n");
    out.push_str("disposition = \"managed\"\nprune = false\nsensitive = true\n");

    out.push_str("\n[file_delivery]\n");
    out.push_str(&format!("branch = {:?}\n", state.file_delivery.branch));
    let reviewers_toml: Vec<String> = state
        .file_delivery
        .reviewers
        .iter()
        .map(|r| format!("{r:?}"))
        .collect();
    out.push_str(&format!("reviewers = [{}]\n", reviewers_toml.join(", ")));
    out.push_str(&format!(
        "commit_message_prefix = {:?}\n",
        state.file_delivery.commit_message_prefix
    ));

    for sys in &state.systems {
        out.push('\n');
        out.push_str("[[systems]]\n");
        out.push_str(&format!("id = {:?}\n", sys.id));
        out.push_str(&format!("name = {:?}\n", sys.name));
        out.push_str("match_prefix = true\n");
        if !state.exclude_patterns.is_empty() {
            let exclude_toml: Vec<String> = state
                .exclude_patterns
                .iter()
                .map(|p| format!("{p:?}"))
                .collect();
            out.push_str(&format!("exclude = [{}]\n", exclude_toml.join(", ")));
        }
    }

    out
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
    println!("    ward plan                    - preview changes");
    println!("    ward apply                   - apply changes");
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
    use crate::config::Manifest;
    use crate::config::manifest::ManagementDisposition;

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

    #[test]
    fn default_manifest_is_canonical_and_parseable() {
        let manifest: Manifest = toml::from_str(EXAMPLE_MANIFEST).unwrap();
        let security = manifest.categories.security.as_ref().unwrap();

        assert_eq!(manifest.schema.version, 2);
        assert!(security.secret_scanning.unwrap());
        assert!(security.secret_scanning_push_protection.unwrap());
        assert!(security.secret_scanning_ai_detection.unwrap());
        assert_eq!(security.policy.disposition, ManagementDisposition::Managed);
        crate::cli::plan::require_canonical_categories(&manifest, "plan").unwrap();
        crate::cli::plan::require_canonical_categories(&manifest, "apply").unwrap();
    }

    #[test]
    fn wizard_manifest_preserves_selected_settings_in_categories() {
        let state = WizardState {
            org: "example-org".to_owned(),
            repo_count: 2,
            security: SecuritySettings {
                secret_scanning: false,
                push_protection: true,
                dependabot_alerts: false,
                dependabot_security_updates: true,
            },
            branch_protection: BranchProtectionSettings {
                enabled: false,
                required_approvals: 3,
                dismiss_stale_reviews: true,
            },
            systems: vec![SystemEntry {
                id: "payments".to_owned(),
                name: "Payments".to_owned(),
                repo_count: 2,
            }],
            exclude_patterns: vec!["operations?".to_owned()],
            file_delivery: FileDeliverySettings {
                branch: "chore/ward".to_owned(),
                reviewers: vec!["octocat".to_owned()],
                commit_message_prefix: "chore: ".to_owned(),
            },
        };

        let rendered = render_manifest(&state);
        assert!(rendered.contains("[categories.security]"));
        assert!(rendered.contains("[categories.branch_protection.default_branch]"));
        assert!(!rendered.contains("\n[security]"));
        assert!(!rendered.contains("\n[branch_protection]"));

        let manifest: Manifest = toml::from_str(&rendered).unwrap();
        let security = manifest.categories.security.as_ref().unwrap();
        let branch_protection = manifest
            .categories
            .branch_protection
            .as_ref()
            .unwrap()
            .default_branch
            .as_ref()
            .unwrap();

        assert_eq!(manifest.org.name, "example-org");
        assert_eq!(manifest.systems[0].id, "payments");
        assert!(manifest.systems[0].match_prefix);
        assert!(!security.secret_scanning.unwrap());
        assert!(security.secret_scanning_push_protection.unwrap());
        assert!(!security.dependabot_alerts.unwrap());
        assert!(security.dependabot_security_updates.unwrap());
        assert!(!branch_protection.enabled);
        assert_eq!(branch_protection.required_approvals, 3);
        assert!(branch_protection.dismiss_stale_reviews);
        crate::cli::plan::require_canonical_categories(&manifest, "plan").unwrap();
        crate::cli::plan::require_canonical_categories(&manifest, "apply").unwrap();
        assert_eq!(manifest.file_delivery.branch, "chore/ward");
        assert_eq!(manifest.file_delivery.reviewers, ["octocat"]);
        assert_eq!(manifest.file_delivery.commit_message_prefix, "chore: ");
    }
}
