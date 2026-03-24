use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::CrosstermBackend,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Tabs, Wrap},
};
use tokio::sync::mpsc;

use crate::cache::{self, CachedRepoEntry, DEFAULT_MAX_AGE, DiskCache};
use crate::config::manifest::BranchProtectionConfig;
use crate::config::templates::load_templates_with_custom_dir;
use crate::config::{Manifest, SecurityCheck, SecurityConfig};
use crate::detection::project_type::ProjectType;
use crate::github::Client;
use crate::github::commits::CommitFile;
use crate::github::repos::Repository;
use crate::github::security::SecurityState;

const SPINNER_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

enum Tab {
    Repos,
    Security,
    Actions,
    Help,
}

enum BgMessage {
    ReposLoaded(Vec<RepoEntry>),
    SecurityLoaded(usize, SecurityState),
    SecurityApplied(String, std::result::Result<(), String>),
    ProtectionApplied(String, std::result::Result<(), String>),
    TemplateDeployed(String, String, std::result::Result<String, String>),
    SettingsApplied(String, std::result::Result<String, String>),
    CustomCheckLoaded(usize, usize, bool),
    Error(String),
}

enum PendingAction {
    ApplySecurity(String),
    ApplyProtection(String),
    SelectTemplate(String),
    DeployTemplate(String, String),
    ApplySettings(String),
    BulkApplySecurity,
}

struct App {
    tab: Tab,
    repos: Vec<RepoEntry>,
    list_state: ListState,
    filter: String,
    is_filtering: bool,
    should_quit: bool,
    status_msg: String,
    systems: Vec<(String, String)>,
    selected_system: usize,
    loading: bool,
    spinner_frame: usize,
    pending_confirm: Option<PendingAction>,
    cache: std::collections::HashMap<String, Vec<RepoEntry>>,
    bulk_progress: Option<(usize, usize)>,
    /// Custom security check definitions from ward.toml `[[security.checks]]`.
    custom_checks: Vec<SecurityCheck>,
    /// Optional persistent disk cache.
    disk_cache: Option<DiskCache>,
}

#[derive(Clone)]
struct RepoEntry {
    repo: Repository,
    security: Option<SecurityState>,
    /// Results for custom checks (indexed same as App::custom_checks).
    /// `None` = still loading, `Some(true)` = pass, `Some(false)` = fail.
    custom_checks: Vec<Option<bool>>,
}

impl App {
    fn new(systems: Vec<(String, String)>, custom_checks: Vec<SecurityCheck>) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            tab: Tab::Repos,
            repos: Vec::new(),
            list_state: state,
            filter: String::new(),
            is_filtering: false,
            should_quit: false,
            status_msg: "Press Enter to load repos".to_owned(),
            systems,
            selected_system: 0,
            loading: false,
            spinner_frame: 0,
            pending_confirm: None,
            cache: std::collections::HashMap::new(),
            bulk_progress: None,
            custom_checks,
            disk_cache: DiskCache::new(),
        }
    }

    fn filtered_repos(&self) -> Vec<&RepoEntry> {
        if self.filter.is_empty() {
            return self.repos.iter().collect();
        }

        let terms: Vec<&str> = self.filter.split_whitespace().collect();
        let includes: Vec<&str> = terms
            .iter()
            .filter(|t| !t.starts_with('!'))
            .copied()
            .collect();
        let excludes: Vec<String> = terms
            .iter()
            .filter(|t| t.starts_with('!'))
            .map(|t| t[1..].to_lowercase())
            .collect();

        self.repos
            .iter()
            .filter(|r| {
                let name = r.repo.name.to_lowercase();
                let include_ok = includes.is_empty()
                    || includes
                        .iter()
                        .any(|inc| name.contains(&inc.to_lowercase()));
                let exclude_ok = excludes.iter().all(|exc| !name.contains(exc.as_str()));
                include_ok && exclude_ok
            })
            .collect()
    }

    fn selected_repo(&self) -> Option<&RepoEntry> {
        let filtered = self.filtered_repos();
        self.list_state
            .selected()
            .and_then(|i| filtered.get(i).copied())
    }

    fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    fn is_loading_security(&self) -> bool {
        !self.repos.is_empty()
            && self
                .repos
                .iter()
                .any(|r| r.security.is_none() || r.custom_checks.iter().any(Option::is_none))
    }
}

pub async fn run(client: &Client, manifest: &Manifest) -> Result<()> {
    let systems: Vec<(String, String)> = manifest
        .systems
        .iter()
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();

    if systems.is_empty() {
        anyhow::bail!("No systems defined in ward.toml - add [[systems]] entries first");
    }

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(systems, manifest.security.checks.clone());

    let result = run_loop(&mut terminal, &mut app, client, manifest).await;

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn spawn_repo_load(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    manifest: &Manifest,
    system_id: &str,
    num_custom_checks: usize,
) {
    let tx = tx.clone();
    let org = client.org.clone();
    let http = client.http.clone();
    let base_url = client.base_url.clone();
    let semaphore = client.semaphore.clone();
    let excludes = manifest.exclude_patterns_for_system(system_id);
    let explicit = manifest.explicit_repos_for_system(system_id);
    let sys_id = system_id.to_owned();

    tokio::spawn(async move {
        let bg_client = Client {
            http,
            org,
            semaphore,
            base_url,
        };

        match bg_client
            .list_repos_for_system(&sys_id, &excludes, &explicit)
            .await
        {
            Ok(repos) => {
                let entries: Vec<RepoEntry> = repos
                    .into_iter()
                    .map(|repo| RepoEntry {
                        repo,
                        security: None,
                        custom_checks: vec![None; num_custom_checks],
                    })
                    .collect();
                let _ = tx.send(BgMessage::ReposLoaded(entries));
            }
            Err(e) => {
                let _ = tx.send(BgMessage::Error(e.to_string()));
            }
        }
    });
}

/// Convert in-memory `RepoEntry` slice to `CachedRepoEntry` and persist silently.
fn save_to_disk_cache(disk_cache: &Option<DiskCache>, system_id: &str, repos: &[RepoEntry]) {
    let entries: Vec<CachedRepoEntry> = repos
        .iter()
        .map(|re| CachedRepoEntry {
            repo: re.repo.clone(),
            security: re.security.clone(),
        })
        .collect();
    cache::try_save(disk_cache, system_id, &entries);
}

fn spawn_security_load(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repos: &[RepoEntry],
) {
    for (idx, entry) in repos.iter().enumerate() {
        let tx = tx.clone();
        let http = client.http.clone();
        let org = client.org.clone();
        let base_url = client.base_url.clone();
        let semaphore = client.semaphore.clone();
        let repo_name = entry.repo.name.clone();
        let repo_data = entry
            .repo
            .security_and_analysis
            .as_ref()
            .map(|sa| serde_json::json!({ "security_and_analysis": sa }));

        tokio::spawn(async move {
            let bg_client = Client {
                http,
                org,
                semaphore,
                base_url,
            };

            let result = bg_client
                .get_security_state_with_repo_data(&repo_name, repo_data.as_ref())
                .await;
            if let Ok(state) = result {
                let _ = tx.send(BgMessage::SecurityLoaded(idx, state));
            }
        });
    }
}

fn spawn_custom_checks(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repos: &[RepoEntry],
    checks: &[SecurityCheck],
) {
    if checks.is_empty() {
        return;
    }

    for (repo_idx, entry) in repos.iter().enumerate() {
        for (check_idx, check) in checks.iter().enumerate() {
            let tx = tx.clone();
            let repo = entry.repo.clone();
            let check = check.clone();
            let http = client.http.clone();
            let org = client.org.clone();
            let base_url = client.base_url.clone();
            let semaphore = client.semaphore.clone();

            tokio::spawn(async move {
                let bg_client = Client {
                    http,
                    org,
                    semaphore,
                    base_url,
                };
                let result = bg_client.run_custom_check(&repo, &check).await;
                let _ = tx.send(BgMessage::CustomCheckLoaded(repo_idx, check_idx, result));
            });
        }
    }
}

fn spawn_security_apply(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repo_name: &str,
    config: &SecurityConfig,
) {
    let tx = tx.clone();
    let http = client.http.clone();
    let org = client.org.clone();
    let base_url = client.base_url.clone();
    let semaphore = client.semaphore.clone();
    let repo = repo_name.to_owned();
    let cfg = config.clone();

    tokio::spawn(async move {
        let bg_client = Client {
            http,
            org,
            semaphore,
            base_url,
        };

        let result = async {
            if cfg.dependabot_alerts {
                bg_client.enable_dependabot_alerts(&repo).await?;
            }
            if cfg.dependabot_security_updates {
                bg_client.enable_dependabot_security_updates(&repo).await?;
            }
            bg_client
                .set_security_features(
                    &repo,
                    cfg.secret_scanning,
                    cfg.secret_scanning_ai_detection,
                    cfg.push_protection,
                )
                .await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let msg = match result {
            Ok(()) => BgMessage::SecurityApplied(repo, Ok(())),
            Err(e) => BgMessage::SecurityApplied(repo, Err(e.to_string())),
        };
        let _ = tx.send(msg);
    });
}

fn spawn_protection_apply(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repo: &str,
    branch: &str,
    config: &BranchProtectionConfig,
) {
    let tx = tx.clone();
    let http = client.http.clone();
    let org = client.org.clone();
    let base_url = client.base_url.clone();
    let semaphore = client.semaphore.clone();
    let repo = repo.to_owned();
    let branch = branch.to_owned();
    let cfg = config.clone();

    tokio::spawn(async move {
        let bg_client = Client {
            http,
            org,
            semaphore,
            base_url,
        };

        let result = bg_client
            .update_branch_protection(&repo, &branch, &cfg)
            .await;

        let msg = match result {
            Ok(()) => BgMessage::ProtectionApplied(repo, Ok(())),
            Err(e) => BgMessage::ProtectionApplied(repo, Err(e.to_string())),
        };
        let _ = tx.send(msg);
    });
}

fn spawn_template_deploy(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    manifest: &Manifest,
    repo_name: &str,
    default_branch: &str,
    template_name: &str,
) {
    let tx = tx.clone();
    let http = client.http.clone();
    let org = client.org.clone();
    let base_url = client.base_url.clone();
    let semaphore = client.semaphore.clone();
    let repo = repo_name.to_owned();
    let default_br = default_branch.to_owned();
    let template = template_name.to_owned();
    let branch_name = manifest.templates.branch.clone();
    let reviewers = manifest.templates.reviewers.clone();
    let commit_prefix = manifest.templates.commit_message_prefix.clone();
    let custom_dir = manifest.templates.custom_dir.clone();
    let registries = manifest.templates.registries.clone();

    tokio::spawn(async move {
        let bg_client = Client {
            http,
            org,
            semaphore,
            base_url,
        };

        let result = async {
            let (target_path, template_category) = match template.as_str() {
                "dependabot" => (".github/dependabot.yml", "dependabot"),
                "codeql" => (".github/workflows/codeql.yml", "codeql"),
                "dependency-submission" => (
                    ".github/workflows/dependency-submission.yml",
                    "dependency-submission",
                ),
                _ => return Err(format!("Unknown template: {template}")),
            };

            // Detect project type
            let project_type = detect_project_type_bg(&bg_client, &repo)
                .await
                .map_err(|e| format!("Detection failed: {e}"))?;

            let tera_template_name = match (&project_type, template_category) {
                (ProjectType::Gradle, "dependabot") => "dependabot/gradle.yml.tera",
                (ProjectType::Npm, "dependabot") => "dependabot/npm.yml.tera",
                (ProjectType::Gradle, "codeql") => "codeql/gradle.yml.tera",
                (ProjectType::Npm, "codeql") => "codeql/npm.yml.tera",
                (ProjectType::Gradle, "dependency-submission") => {
                    "dependency-submission/gradle.yml.tera"
                }
                (pt, cat) => {
                    return Err(format!("No template for {cat} + {pt} in {repo}"));
                }
            };

            let mut ctx = tera::Context::new();
            ctx.insert("default_branch", &default_br);

            match project_type {
                ProjectType::Gradle => {
                    let java_ver = detect_java_version_bg(&bg_client, &repo)
                        .await
                        .map_err(|e| e.to_string())?;
                    ctx.insert("java_version", &java_ver.to_string());
                    if let Some(reg) = registries.get("gradle-artifactory") {
                        ctx.insert("registry_url", &reg.url);
                        if let Some(ref provider) = reg.jfrog_oidc_provider {
                            ctx.insert("jfrog_oidc_provider", provider);
                        }
                    }
                }
                ProjectType::Npm => {
                    let node_ver = detect_node_version_bg(&bg_client, &repo)
                        .await
                        .map_err(|e| e.to_string())?;
                    ctx.insert("node_version", &node_ver);
                }
                _ => {}
            }

            let tera =
                load_templates_with_custom_dir(custom_dir.as_deref().map(std::path::Path::new))
                    .map_err(|e| format!("Template load error: {e}"))?;
            let rendered = tera
                .render(tera_template_name, &ctx)
                .map_err(|e| format!("Render error: {e}"))?;

            // Check if file already exists and matches
            if let Ok(Some(existing)) = bg_client.get_file(&repo, target_path, None).await
                && let Ok(decoded) = Client::decode_content(&existing)
                && decoded.trim() == rendered.trim()
            {
                return Err(format!("{repo}: already up to date"));
            }

            bg_client
                .create_branch(&repo, &branch_name, &default_br)
                .await
                .map_err(|e| format!("Branch error: {e}"))?;

            let message = format!("{commit_prefix}add {template} configuration");
            let files = vec![CommitFile {
                path: target_path.to_owned(),
                content: rendered,
            }];
            bg_client
                .create_commit(&repo, &branch_name, &message, &files)
                .await
                .map_err(|e| format!("Commit error: {e}"))?;

            let pr_title = format!("{commit_prefix}add {template} configuration");
            let pr_body = format!(
                "## Ward: automated template commit\n\n\
                 Template: `{template}`\nFile: `{target_path}`\n\n\
                 This PR was created by [ward](https://github.com/OriginalMHV/ward).\n\n\
                 ---\n*Review the file contents, then merge.*"
            );
            let pr = bg_client
                .create_pull_request(
                    &repo,
                    &pr_title,
                    &pr_body,
                    &branch_name,
                    &default_br,
                    &reviewers,
                )
                .await
                .map_err(|e| format!("PR error: {e}"))?;

            Ok(pr.html_url)
        }
        .await;

        let _ = tx.send(BgMessage::TemplateDeployed(repo, template, result));
    });
}

fn spawn_settings_apply(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    manifest: &Manifest,
    repo_name: &str,
    default_branch: &str,
) {
    let tx = tx.clone();
    let http = client.http.clone();
    let org = client.org.clone();
    let base_url = client.base_url.clone();
    let semaphore = client.semaphore.clone();
    let repo = repo_name.to_owned();
    let default_br = default_branch.to_owned();
    let branch_name = manifest.templates.branch.clone();
    let reviewers = manifest.templates.reviewers.clone();
    let commit_prefix = manifest.templates.commit_message_prefix.clone();
    let custom_dir = manifest.templates.custom_dir.clone();
    let is_ops = repo.ends_with("-operation")
        || repo.ends_with("-operations")
        || repo.ends_with("-ops")
        || repo.ends_with("-gitops");

    tokio::spawn(async move {
        let bg_client = Client {
            http,
            org,
            semaphore,
            base_url,
        };

        let result = async {
            let mut actions: Vec<String> = Vec::new();

            // Check and create copilot review ruleset
            let rulesets = bg_client
                .list_rulesets(&repo)
                .await
                .map_err(|e| format!("List rulesets: {e}"))?;
            let has_copilot_review = rulesets.iter().any(|r| r.name == "Copilot Code Review");
            if !has_copilot_review {
                bg_client
                    .create_copilot_review_ruleset(&repo)
                    .await
                    .map_err(|e| format!("Create ruleset: {e}"))?;
                actions.push("ruleset created".to_owned());
            }

            // Check and deploy copilot instructions
            let has_instructions = bg_client
                .get_file(&repo, ".github/copilot-instructions.md", None)
                .await
                .map_err(|e| format!("Check instructions: {e}"))?
                .is_some();

            if !has_instructions {
                let template_name = if is_ops {
                    "copilot-review/instructions-ops.md.tera"
                } else {
                    "copilot-review/instructions-app.md.tera"
                };

                let tera =
                    load_templates_with_custom_dir(custom_dir.as_deref().map(std::path::Path::new))
                        .map_err(|e| format!("Template load: {e}"))?;
                let rendered = tera
                    .render(template_name, &tera::Context::new())
                    .map_err(|e| format!("Render: {e}"))?;

                bg_client
                    .create_branch(&repo, &branch_name, &default_br)
                    .await
                    .map_err(|e| format!("Branch: {e}"))?;

                let files = vec![CommitFile {
                    path: ".github/copilot-instructions.md".to_owned(),
                    content: rendered,
                }];
                bg_client
                    .create_commit(
                        &repo,
                        &branch_name,
                        &format!("{commit_prefix}add Copilot review instructions"),
                        &files,
                    )
                    .await
                    .map_err(|e| format!("Commit: {e}"))?;

                let pr = bg_client
                    .create_pull_request(
                        &repo,
                        &format!("{commit_prefix}add Copilot review instructions"),
                        "## Ward: Copilot review instructions\n\n\
                         Deploys `.github/copilot-instructions.md` for Copilot code review.\n\n\
                         ---\n*Review the instructions, then merge.*",
                        &branch_name,
                        &default_br,
                        &reviewers,
                    )
                    .await
                    .map_err(|e| format!("PR: {e}"))?;
                actions.push(format!("instructions PR: {}", pr.html_url));
            }

            if actions.is_empty() {
                Ok("already up to date".to_owned())
            } else {
                Ok(actions.join("; "))
            }
        }
        .await;

        let _ = tx.send(BgMessage::SettingsApplied(repo, result));
    });
}

async fn detect_project_type_bg(client: &Client, repo: &str) -> Result<ProjectType> {
    if client
        .get_file(repo, "build.gradle.kts", None)
        .await?
        .is_some()
    {
        return Ok(ProjectType::Gradle);
    }
    if client.get_file(repo, "build.gradle", None).await?.is_some() {
        return Ok(ProjectType::Gradle);
    }
    if client.get_file(repo, "package.json", None).await?.is_some() {
        return Ok(ProjectType::Npm);
    }
    if client.get_file(repo, "Cargo.toml", None).await?.is_some() {
        return Ok(ProjectType::Cargo);
    }
    Ok(ProjectType::Unknown)
}

async fn detect_java_version_bg(client: &Client, repo: &str) -> Result<u8> {
    for file in &["build.gradle.kts", "build.gradle"] {
        if let Some(content) = client.get_file(repo, file, None).await? {
            let text = Client::decode_content(&content)?;
            if let Some(ver) = crate::detection::versions::extract_java_version(&text) {
                return Ok(ver);
            }
        }
    }
    Ok(21)
}

async fn detect_node_version_bg(client: &Client, repo: &str) -> Result<String> {
    if let Some(content) = client.get_file(repo, "package.json", None).await? {
        let text = Client::decode_content(&content)?;
        if let Some(ver) = crate::detection::versions::extract_node_version(&text) {
            let major: String = ver.chars().filter(|c| c.is_ascii_digit()).collect();
            if !major.is_empty() {
                return Ok(major);
            }
        }
    }
    Ok("20".to_owned())
}

fn switch_to_cached_or_prompt(app: &mut App) {
    let (sys_id, sys_name) = &app.systems[app.selected_system];
    if let Some(cached) = app.cache.get(sys_id) {
        app.repos = cached.clone();
        app.list_state.select(Some(0));
        app.status_msg = format!("{} repos for {sys_name} (cached)", app.repos.len());
    } else if let Some(disk_hit) = app
        .disk_cache
        .as_ref()
        .and_then(|dc| dc.load(sys_id, DEFAULT_MAX_AGE))
    {
        let age_str = cache::format_age(&disk_hit.cached_at);
        let num_checks = app.custom_checks.len();
        let entries: Vec<RepoEntry> = disk_hit
            .repos
            .into_iter()
            .map(|ce| RepoEntry {
                repo: ce.repo,
                security: ce.security,
                custom_checks: vec![None; num_checks],
            })
            .collect();
        let count = entries.len();
        app.repos = entries.clone();
        app.cache.insert(sys_id.clone(), entries);
        app.list_state.select(Some(0));
        app.status_msg = format!("{count} repos for {sys_name} (cached, {age_str})");
    } else {
        app.repos.clear();
        app.list_state.select(Some(0));
        app.status_msg = format!("{sys_name} ({sys_id}). Press Enter to load.");
    }
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &Client,
    manifest: &Manifest,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<BgMessage>();

    loop {
        // Advance spinner each render cycle when loading
        if app.loading || app.is_loading_security() {
            app.spinner_frame = app.spinner_frame.wrapping_add(1);
        }

        terminal.draw(|f| draw(f, app))?;

        // Check for background task results (non-blocking)
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BgMessage::ReposLoaded(entries) => {
                    let count = entries.len();
                    let (sys_id, sys_name) = &app.systems[app.selected_system];
                    app.status_msg =
                        format!("Loaded {count} repos for {sys_name}. Fetching security...");
                    app.repos = entries;
                    app.list_state.select(Some(0));
                    app.loading = false;
                    app.cache.insert(sys_id.clone(), app.repos.clone());
                    save_to_disk_cache(&app.disk_cache, sys_id, &app.repos);
                    spawn_security_load(&tx, client, &app.repos);
                    spawn_custom_checks(&tx, client, &app.repos, &app.custom_checks);
                }
                BgMessage::SecurityLoaded(idx, state) => {
                    if let Some(entry) = app.repos.get_mut(idx) {
                        entry.security = Some(state);
                    }
                    let loaded = app.repos.iter().filter(|r| r.security.is_some()).count();
                    let total = app.repos.len();
                    if loaded == total
                        && app
                            .repos
                            .iter()
                            .all(|r| r.custom_checks.iter().all(Option::is_some))
                    {
                        let (sys_id, sys_name) = &app.systems[app.selected_system];
                        app.status_msg = format!("{total} repos loaded for {sys_name}");
                        app.cache.insert(sys_id.clone(), app.repos.clone());
                        // Update disk cache now that security data is complete.
                        save_to_disk_cache(&app.disk_cache, sys_id, &app.repos);
                    } else {
                        app.status_msg =
                            format!("[{}] Security: {loaded}/{total}...", app.spinner_char());
                    }
                }
                BgMessage::CustomCheckLoaded(repo_idx, check_idx, result) => {
                    if let Some(entry) = app.repos.get_mut(repo_idx) {
                        if let Some(slot) = entry.custom_checks.get_mut(check_idx) {
                            *slot = Some(result);
                        }
                    }
                    // Refresh cache & status when everything is done
                    let all_security = app.repos.iter().all(|r| r.security.is_some());
                    let all_custom = app
                        .repos
                        .iter()
                        .all(|r| r.custom_checks.iter().all(Option::is_some));
                    if all_security && all_custom {
                        let total = app.repos.len();
                        let (sys_id, sys_name) = &app.systems[app.selected_system];
                        app.status_msg = format!("{total} repos loaded for {sys_name}");
                        app.cache.insert(sys_id.clone(), app.repos.clone());
                    }
                }
                BgMessage::SecurityApplied(repo, result) => {
                    let spinner = app.spinner_char();
                    match result {
                        Ok(()) => {
                            app.status_msg = format!("Applied security to {repo}");
                            if let Some((ref mut done, total)) = app.bulk_progress {
                                *done += 1;
                                if *done >= total {
                                    app.status_msg =
                                        format!("Bulk security complete: {total}/{total}");
                                    app.bulk_progress = None;
                                    app.loading = false;
                                } else {
                                    app.status_msg =
                                        format!("[{spinner}] Bulk security: {done}/{total}...");
                                }
                            } else {
                                app.loading = false;
                            }
                        }
                        Err(e) => {
                            app.status_msg = format!("Failed to apply to {repo}: {e}");
                            if let Some((ref mut done, total)) = app.bulk_progress {
                                *done += 1;
                                if *done >= total {
                                    app.status_msg =
                                        format!("Bulk security done ({total}), last error: {e}");
                                    app.bulk_progress = None;
                                    app.loading = false;
                                }
                            } else {
                                app.loading = false;
                            }
                        }
                    }
                }
                BgMessage::ProtectionApplied(repo, result) => {
                    match result {
                        Ok(()) => {
                            app.status_msg = format!("Applied branch protection to {repo}");
                        }
                        Err(e) => {
                            app.status_msg = format!("Failed branch protection on {repo}: {e}");
                        }
                    }
                    app.loading = false;
                }
                BgMessage::TemplateDeployed(repo, template, result) => {
                    match result {
                        Ok(pr_url) => {
                            app.status_msg = format!("Deployed {template} to {repo}: {pr_url}");
                        }
                        Err(e) => {
                            app.status_msg = format!("Template {template} failed on {repo}: {e}");
                        }
                    }
                    app.loading = false;
                }
                BgMessage::SettingsApplied(repo, result) => {
                    match result {
                        Ok(detail) => {
                            app.status_msg = format!("Settings applied to {repo}: {detail}");
                        }
                        Err(e) => {
                            app.status_msg = format!("Settings failed on {repo}: {e}");
                        }
                    }
                    app.loading = false;
                }
                BgMessage::Error(e) => {
                    app.status_msg = format!("Error: {e}");
                    app.loading = false;
                }
            }
        }

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Confirmation mode: only y/n accepted (and template sub-menu keys)
            if app.pending_confirm.is_some() {
                match key.code {
                    KeyCode::Char('y') => {
                        let action = app.pending_confirm.take();
                        match action {
                            Some(PendingAction::ApplySecurity(repo)) => {
                                let sys_id = &app.systems[app.selected_system].0;
                                let sec_config = manifest.security_for_system(sys_id).clone();
                                app.loading = true;
                                app.status_msg = format!(
                                    "[{}] Applying security to {repo}...",
                                    app.spinner_char()
                                );
                                spawn_security_apply(&tx, client, &repo, &sec_config);
                            }
                            Some(PendingAction::ApplyProtection(repo)) => {
                                if let Some(entry) = app.repos.iter().find(|e| e.repo.name == repo)
                                {
                                    let branch = entry.repo.default_branch.clone();
                                    let cfg = manifest.branch_protection.clone();
                                    app.loading = true;
                                    app.status_msg = format!(
                                        "[{}] Applying protection to {repo}...",
                                        app.spinner_char()
                                    );
                                    spawn_protection_apply(&tx, client, &repo, &branch, &cfg);
                                }
                            }
                            Some(PendingAction::DeployTemplate(repo, template)) => {
                                if let Some(entry) = app.repos.iter().find(|e| e.repo.name == repo)
                                {
                                    let default_br = entry.repo.default_branch.clone();
                                    app.loading = true;
                                    app.status_msg = format!(
                                        "[{}] Deploying {template} to {repo}...",
                                        app.spinner_char()
                                    );
                                    spawn_template_deploy(
                                        &tx,
                                        client,
                                        manifest,
                                        &repo,
                                        &default_br,
                                        &template,
                                    );
                                }
                            }
                            Some(PendingAction::ApplySettings(repo)) => {
                                if let Some(entry) = app.repos.iter().find(|e| e.repo.name == repo)
                                {
                                    let default_br = entry.repo.default_branch.clone();
                                    app.loading = true;
                                    app.status_msg = format!(
                                        "[{}] Applying settings to {repo}...",
                                        app.spinner_char()
                                    );
                                    spawn_settings_apply(&tx, client, manifest, &repo, &default_br);
                                }
                            }
                            Some(PendingAction::BulkApplySecurity) => {
                                let filtered: Vec<String> = app
                                    .filtered_repos()
                                    .iter()
                                    .map(|e| e.repo.name.clone())
                                    .collect();
                                let total = filtered.len();
                                let sys_id = &app.systems[app.selected_system].0;
                                let sec_config = manifest.security_for_system(sys_id).clone();
                                app.loading = true;
                                app.bulk_progress = Some((0, total));
                                app.status_msg =
                                    format!("[{}] Bulk security: 0/{total}...", app.spinner_char());
                                for repo in &filtered {
                                    spawn_security_apply(&tx, client, repo, &sec_config);
                                }
                            }
                            Some(PendingAction::SelectTemplate(_)) | None => {}
                        }
                    }
                    // Template sub-menu key handling
                    KeyCode::Char('d') => {
                        if let Some(PendingAction::SelectTemplate(repo)) =
                            app.pending_confirm.take()
                        {
                            app.pending_confirm = Some(PendingAction::DeployTemplate(
                                repo.clone(),
                                "dependabot".to_owned(),
                            ));
                            app.status_msg = format!("Deploy dependabot to {repo}? (y/n)");
                        }
                    }
                    KeyCode::Char('c') => {
                        if let Some(PendingAction::SelectTemplate(repo)) =
                            app.pending_confirm.take()
                        {
                            app.pending_confirm = Some(PendingAction::DeployTemplate(
                                repo.clone(),
                                "codeql".to_owned(),
                            ));
                            app.status_msg = format!("Deploy codeql to {repo}? (y/n)");
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(PendingAction::SelectTemplate(repo)) =
                            app.pending_confirm.take()
                        {
                            app.pending_confirm = Some(PendingAction::DeployTemplate(
                                repo.clone(),
                                "dependency-submission".to_owned(),
                            ));
                            app.status_msg =
                                format!("Deploy dependency-submission to {repo}? (y/n)");
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        app.pending_confirm = None;
                        app.status_msg = "Cancelled.".to_owned();
                    }
                    _ => {}
                }
                continue;
            }

            if app.is_filtering {
                match key.code {
                    KeyCode::Esc => {
                        app.is_filtering = false;
                        app.filter.clear();
                    }
                    KeyCode::Enter => {
                        app.is_filtering = false;
                    }
                    KeyCode::Backspace => {
                        app.filter.pop();
                    }
                    KeyCode::Char(c) => {
                        app.filter.push(c);
                        app.list_state.select(Some(0));
                    }
                    _ => {}
                }
                continue;
            }

            match key.code {
                KeyCode::Char('q') => app.should_quit = true,
                KeyCode::Char('1') => app.tab = Tab::Repos,
                KeyCode::Char('2') => app.tab = Tab::Security,
                KeyCode::Char('3') => app.tab = Tab::Actions,
                KeyCode::Char('?') => app.tab = Tab::Help,
                KeyCode::Char('/') => {
                    app.is_filtering = true;
                    app.filter.clear();
                }
                KeyCode::Char('a') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::ApplySecurity(repo.clone()));
                        app.status_msg = format!("Apply security to {repo}? (y/n)");
                    }
                }
                KeyCode::Char('p') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::ApplyProtection(repo.clone()));
                        app.status_msg = format!("Apply branch protection to {repo}? (y/n)");
                    }
                }
                KeyCode::Char('t') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::SelectTemplate(repo));
                        app.status_msg =
                            "Deploy template: (d)ependabot (c)odeql (s)ubmission".to_owned();
                    }
                }
                KeyCode::Char('S') => {
                    if let Some(entry) = app.selected_repo() {
                        let repo = entry.repo.name.clone();
                        app.pending_confirm = Some(PendingAction::ApplySettings(repo.clone()));
                        app.status_msg = format!(
                            "Apply settings (copilot ruleset + instructions) to {repo}? (y/n)"
                        );
                    }
                }
                KeyCode::Char('A') => {
                    let count = app.filtered_repos().len();
                    if count > 0 {
                        app.pending_confirm = Some(PendingAction::BulkApplySecurity);
                        app.status_msg =
                            format!("Apply security to all {count} filtered repos? (y/n)");
                    }
                }
                KeyCode::Char('r') => {
                    if !app.loading {
                        app.loading = true;
                        let sys_id = &app.systems[app.selected_system].0;
                        app.status_msg = format!(
                            "[{}] Loading {}...",
                            app.spinner_char(),
                            app.systems[app.selected_system].1
                        );
                        spawn_repo_load(&tx, client, manifest, sys_id, app.custom_checks.len());
                    }
                }
                KeyCode::Char('R') => {
                    if !app.loading {
                        let sys_id = app.systems[app.selected_system].0.clone();
                        app.cache.remove(&sys_id);
                        cache::try_invalidate(&app.disk_cache, &sys_id);
                        app.loading = true;
                        app.status_msg = format!(
                            "[{}] Force loading {}...",
                            app.spinner_char(),
                            app.systems[app.selected_system].1
                        );
                        spawn_repo_load(&tx, client, manifest, &sys_id, app.custom_checks.len());
                    }
                }
                KeyCode::Char('l') | KeyCode::Enter => {
                    if !app.loading {
                        app.loading = true;
                        let sys_id = &app.systems[app.selected_system].0;
                        app.status_msg = format!(
                            "[{}] Loading {}...",
                            app.spinner_char(),
                            app.systems[app.selected_system].1
                        );
                        spawn_repo_load(&tx, client, manifest, sys_id, app.custom_checks.len());
                    }
                }
                KeyCode::Tab | KeyCode::Char('s') => {
                    if !app.systems.is_empty() {
                        app.selected_system = (app.selected_system + 1) % app.systems.len();
                        switch_to_cached_or_prompt(app);
                    }
                }
                KeyCode::BackTab => {
                    if !app.systems.is_empty() {
                        app.selected_system = if app.selected_system == 0 {
                            app.systems.len() - 1
                        } else {
                            app.selected_system - 1
                        };
                        switch_to_cached_or_prompt(app);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let len = app.filtered_repos().len();
                    if len > 0 {
                        let i = app.list_state.selected().unwrap_or(0);
                        app.list_state.select(Some((i + 1).min(len - 1)));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let i = app.list_state.selected().unwrap_or(0);
                    app.list_state.select(Some(i.saturating_sub(1)));
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);

    match app.tab {
        Tab::Repos => draw_repos_tab(f, chunks[1], app),
        Tab::Security => draw_security_tab(f, chunks[1], app),
        Tab::Actions => draw_actions_tab(f, chunks[1]),
        Tab::Help => draw_help_tab(f, chunks[1]),
    }

    draw_status(f, chunks[2], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let titles = vec!["[1] Repos", "[2] Security", "[3] Actions", "[?] Help"];
    let selected = match app.tab {
        Tab::Repos => 0,
        Tab::Security => 1,
        Tab::Actions => 2,
        Tab::Help => 3,
    };

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .title(" Ward: GitHub Repository Management "),
        )
        .select(selected)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(Style::default().fg(Color::Cyan).bold());

    f.render_widget(tabs, area);
}

fn draw_repos_tab(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let filtered = app.filtered_repos();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|entry| {
            let lang = entry.repo.language.as_deref().unwrap_or("-");
            let (indicator, color) = entry
                .security
                .as_ref()
                .map(|s| {
                    if s.dependabot_alerts && s.secret_scanning && s.push_protection {
                        ("[ok]", Color::Green)
                    } else {
                        ("[!!]", Color::Yellow)
                    }
                })
                .unwrap_or(("[..]", Color::DarkGray));

            ListItem::new(Line::from(vec![
                Span::styled(indicator, Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(entry.repo.name.clone(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(lang, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let sys_label = app
        .systems
        .get(app.selected_system)
        .map(|(_, name)| name.as_str())
        .unwrap_or("?");
    let title = if app.loading {
        format!(" {sys_label} [{}] loading... ", app.spinner_char())
    } else if app.is_filtering {
        format!(" {sys_label} (filter: {}_) ", app.filter)
    } else if app.filter.is_empty() {
        format!(" {sys_label} ({}) ", filtered.len())
    } else {
        format!(" {sys_label} ({}/{}) ", filtered.len(), app.repos.len())
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, chunks[0], &mut app.list_state.clone());

    let detail = if let Some(entry) = app.selected_repo() {
        let sec = entry.security.as_ref();
        let icon = |b: bool| -> (&str, Color) {
            if b {
                ("[Y]", Color::Green)
            } else {
                ("[N]", Color::Red)
            }
        };

        let mut lines = vec![
            Line::from(Span::styled(
                entry.repo.name.clone(),
                Style::default().fg(Color::Cyan).bold(),
            )),
            Line::from(""),
        ];

        // Description (truncated to 2 lines)
        if let Some(ref desc) = entry.repo.description
            && !desc.is_empty()
        {
            let max_chars = 80;
            if desc.len() > max_chars {
                let first = &desc[..max_chars];
                let rest = &desc[max_chars..];
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(first, Style::default().fg(Color::DarkGray)),
                ]));
                let trimmed = if rest.len() > max_chars {
                    format!("{}...", &rest[..max_chars.min(rest.len())])
                } else {
                    rest.to_owned()
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(trimmed, Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(desc.clone(), Style::default().fg(Color::DarkGray)),
                ]));
            }
            lines.push(Line::from(""));
        }

        if entry.repo.archived {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("[ARCHIVED]", Style::default().fg(Color::Red).bold()),
            ]));
            lines.push(Line::from(""));
        }

        lines.extend([
            Line::from(vec![
                Span::raw("  Language:   "),
                Span::styled(
                    entry.repo.language.as_deref().unwrap_or("-"),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::raw("  Branch:     "),
                Span::raw(&entry.repo.default_branch),
            ]),
            Line::from(vec![
                Span::raw("  Visibility: "),
                Span::raw(&entry.repo.visibility),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Security",
                Style::default().fg(Color::Cyan).bold(),
            )),
        ]);

        if let Some(s) = sec {
            let features = [
                ("Dependabot Alerts", s.dependabot_alerts),
                ("Security Updates", s.dependabot_security_updates),
                ("Secret Scanning", s.secret_scanning),
                ("AI Detection", s.secret_scanning_ai_detection),
                ("Push Protection", s.push_protection),
            ];
            for (label, enabled) in features {
                let (text, color) = icon(enabled);
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(text, Style::default().fg(color)),
                    Span::raw(format!(" {label}")),
                ]));
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  [..] Loading...",
                Style::default().fg(Color::DarkGray),
            )));
        }

        lines
    } else {
        vec![Line::from("  Select a repository")]
    };

    let detail_widget = Paragraph::new(detail)
        .block(Block::default().borders(Borders::ALL).title(" Details "))
        .wrap(Wrap { trim: false });

    f.render_widget(detail_widget, chunks[1]);
}

fn draw_security_tab(f: &mut Frame, area: Rect, app: &App) {
    let sys_label = app
        .systems
        .get(app.selected_system)
        .map(|(_, name)| name.as_str())
        .unwrap_or("?");
    let sys_id = app
        .systems
        .get(app.selected_system)
        .map(|(id, _)| id.as_str())
        .unwrap_or("");

    if app.repos.is_empty() {
        let msg = Paragraph::new("  Press Enter to load repos, then switch to this tab.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Security: {sys_label} ")),
        );
        f.render_widget(msg, area);
        return;
    }

    // Detect common prefix (system-id + dash) to strip from repo names
    let prefix = format!("{sys_id}-");
    let all_share_prefix = app.repos.iter().all(|e| e.repo.name.starts_with(&prefix));

    let display_name = |name: &str| -> String {
        if all_share_prefix {
            name.strip_prefix(&prefix).unwrap_or(name).to_owned()
        } else {
            name.to_owned()
        }
    };

    // Dynamic column width from longest display name, capped at 50
    let max_name_len = app
        .repos
        .iter()
        .map(|e| display_name(&e.repo.name).len())
        .max()
        .unwrap_or(20);
    let col_repo: usize = max_name_len.clamp(10, 50) + 2;

    let truncate_name = |name: &str| -> String {
        let dname = display_name(name);
        if dname.len() > col_repo {
            format!("{}...", &dname[..col_repo - 3])
        } else {
            dname
        }
    };

    // Build header: built-in columns + custom check columns
    let builtin_headers = [
        "Repository",
        "Dependabot",
        "Secret Scanning",
        "AI Detection",
        "Push Protection",
        "Security Updates",
    ];
    let header_style = Style::default().fg(Color::Cyan).bold();

    let mut header_cells: Vec<Cell> = builtin_headers
        .iter()
        .map(|h| Cell::from(*h).style(header_style))
        .collect();
    for check in &app.custom_checks {
        header_cells.push(Cell::from(check.name()).style(header_style));
    }

    let header = Row::new(header_cells)
        .style(Style::default())
        .bottom_margin(0);

    let mut secured = 0;
    let mut issues = 0;
    let mut pending = 0;

    let rows: Vec<Row> = app
        .repos
        .iter()
        .enumerate()
        .map(|(row_idx, entry)| {
            let row_bg = if row_idx % 2 == 1 {
                Color::Rgb(30, 30, 40)
            } else {
                Color::Reset
            };
            let row_style = Style::default().bg(row_bg);

            let icon = |b: bool| -> Cell {
                if b {
                    Cell::from(" [Y]").style(Style::default().fg(Color::Green).bg(row_bg))
                } else {
                    Cell::from(" [N]").style(Style::default().fg(Color::Red).bg(row_bg))
                }
            };

            let loading_cell =
                Cell::from(" [..]").style(Style::default().fg(Color::DarkGray).bg(row_bg));

            if let Some(s) = entry.security.as_ref() {
                let all_ok = s.dependabot_alerts
                    && s.secret_scanning
                    && s.secret_scanning_ai_detection
                    && s.push_protection;
                if all_ok {
                    secured += 1;
                } else {
                    issues += 1;
                }

                let mut cells = vec![
                    Cell::from(truncate_name(&entry.repo.name)).style(Style::default().bg(row_bg)),
                    icon(s.dependabot_alerts),
                    icon(s.secret_scanning),
                    icon(s.secret_scanning_ai_detection),
                    icon(s.push_protection),
                    icon(s.dependabot_security_updates),
                ];

                // Append custom check cells
                for result in &entry.custom_checks {
                    cells.push(match result {
                        Some(val) => icon(*val),
                        None => loading_cell.clone(),
                    });
                }

                Row::new(cells).style(row_style)
            } else {
                pending += 1;
                let builtin_count = 6; // repo name + 5 built-in checks
                let total_cols = builtin_count + app.custom_checks.len();
                let mut cells = Vec::with_capacity(total_cols);
                cells.push(
                    Cell::from(truncate_name(&entry.repo.name))
                        .style(Style::default().fg(Color::DarkGray).bg(row_bg)),
                );
                for _ in 1..total_cols {
                    cells.push(loading_cell.clone());
                }
                Row::new(cells).style(row_style)
            }
        })
        .collect();

    // Build column widths: built-in + custom
    let mut widths = vec![
        Constraint::Min(col_repo as u16),
        Constraint::Min(12),
        Constraint::Min(17),
        Constraint::Min(14),
        Constraint::Min(17),
        Constraint::Min(18),
    ];
    for check in &app.custom_checks {
        // Width based on check name length, with a minimum of 10
        let w = check.name().len().max(10) + 2;
        widths.push(Constraint::Min(w as u16));
    }

    let prefix_note = if all_share_prefix {
        format!(" (prefix {prefix} stripped)")
    } else {
        String::new()
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Security: {sys_label}{prefix_note} ")),
        )
        .column_spacing(1);

    // Split area: table gets main space, summary gets 1 line at bottom
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(area);

    f.render_widget(table, layout[0]);

    let summary = if pending > 0 {
        format!("  {secured} secured, {issues} issues, {pending} loading...")
    } else {
        format!("  {secured} secured, {issues} need attention")
    };
    let summary_widget = Paragraph::new(Line::from(Span::styled(
        summary,
        Style::default()
            .fg(if issues > 0 {
                Color::Yellow
            } else {
                Color::Green
            })
            .bold(),
    )));
    f.render_widget(summary_widget, layout[1]);
}

fn draw_actions_tab(f: &mut Frame, area: Rect) {
    let key_style = Style::default().fg(Color::Yellow).bold();
    let heading = Style::default().fg(Color::Cyan).bold();
    let dim = Style::default().fg(Color::DarkGray);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Available Actions (on selected repo)",
            heading,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  a", key_style),
            Span::raw("    Apply security settings"),
        ]),
        Line::from(vec![
            Span::styled("  p", key_style),
            Span::raw("    Apply branch protection"),
        ]),
        Line::from(vec![
            Span::styled("  t", key_style),
            Span::raw("    Deploy template (sub-menu: dependabot/codeql/submission)"),
        ]),
        Line::from(vec![
            Span::styled("  S", key_style),
            Span::raw("    Apply settings (copilot ruleset + instructions)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Bulk Actions (on all filtered repos)",
            heading,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  A", key_style),
            Span::raw("    Apply security to all filtered repos"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Navigation", heading)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  r", key_style),
            Span::raw("    Refresh/reload current system"),
        ]),
        Line::from(vec![
            Span::styled("  R", key_style),
            Span::raw("    Force reload (ignore cache)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Templates deploy a branch + commit + PR for the selected repo.",
            dim,
        )),
        Line::from(Span::styled(
            "  Settings creates a copilot review ruleset and instructions file.",
            dim,
        )),
    ];

    let widget =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Actions "));

    f.render_widget(widget, area);
}

fn draw_help_tab(f: &mut Frame, area: Rect) {
    let heading = Style::default().fg(Color::Cyan).bold();
    let dim = Style::default().fg(Color::DarkGray);
    let key = Style::default().fg(Color::Yellow);

    let block = Block::default().borders(Borders::ALL).title(" Help ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Helper: build a section with a heading, separator, and key/desc rows
    fn section<'a>(
        title: &'a str,
        entries: &[(&'a str, &'a str)],
        heading: Style,
        dim: Style,
        key_style: Style,
    ) -> Vec<Line<'a>> {
        let mut lines = vec![
            Line::from(Span::styled(format!(" {title}"), heading)),
            Line::from(Span::styled(
                format!(" {}", "\u{2500}".repeat(title.len())),
                dim,
            )),
        ];
        for &(k, desc) in entries {
            lines.push(Line::from(vec![
                Span::styled(format!(" {k:<15}"), key_style),
                Span::raw(desc),
            ]));
        }
        lines.push(Line::from(""));
        lines
    }

    // Left column
    let mut left: Vec<Line> = Vec::new();
    left.push(Line::from(""));
    left.extend(section(
        "Navigation",
        &[
            ("j/k, arrows", "Move up/down"),
            ("Tab/s", "Next system"),
            ("Shift+Tab", "Previous system"),
            ("Enter/l", "Load repos"),
            ("/", "Filter"),
            ("Esc", "Clear filter"),
        ],
        heading,
        dim,
        key,
    ));
    left.extend(section(
        "General",
        &[
            ("q", "Quit"),
            ("r", "Reload"),
            ("R", "Force reload"),
            ("?", "Help"),
        ],
        heading,
        dim,
        key,
    ));
    left.extend(section(
        "Filter Syntax",
        &[
            ("foo", "Show matching"),
            ("!ops", "Hide matching"),
            ("!ops !sys", "Hide both"),
            ("foo !ops", "Show foo, hide ops"),
        ],
        heading,
        dim,
        key,
    ));
    left.push(Line::from(Span::styled(
        " Stack ! terms like kanban labels to narrow view.",
        dim,
    )));

    // Right column
    let mut right: Vec<Line> = Vec::new();
    right.push(Line::from(""));
    right.extend(section(
        "Repo Actions",
        &[
            ("a", "Apply security"),
            ("p", "Apply branch protection"),
            ("t", "Deploy template (d/c/s)"),
            ("S", "Apply settings (copilot)"),
        ],
        heading,
        dim,
        key,
    ));
    right.extend(section(
        "Bulk Actions",
        &[("A", "Apply security to all")],
        heading,
        dim,
        key,
    ));
    right.extend(section(
        "Tabs",
        &[
            ("1", "Repos"),
            ("2", "Security"),
            ("3", "Actions"),
            ("?", "Help"),
        ],
        heading,
        dim,
        key,
    ));

    let left_widget = Paragraph::new(left);
    let right_widget = Paragraph::new(right);

    f.render_widget(left_widget, columns[0]);
    f.render_widget(right_widget, columns[1]);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let (sys_id, sys_name) = app
        .systems
        .get(app.selected_system)
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .unwrap_or(("?", "none"));

    let sys_idx = format!("[{}/{}]", app.selected_system + 1, app.systems.len());
    let repo_count = if app.repos.is_empty() {
        String::new()
    } else {
        format!("{} repos", app.repos.len())
    };

    let status_detail = if app.loading {
        format!("[{}] {}", app.spinner_char(), app.status_msg)
    } else if app.is_loading_security() {
        let loaded = app.repos.iter().filter(|r| r.security.is_some()).count();
        let total = app.repos.len();
        format!("[{}] Security: {loaded}/{total}...", app.spinner_char())
    } else {
        app.status_msg.clone()
    };

    let mut spans = vec![
        Span::styled(" System: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{sys_name} ({sys_id})"),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(format!(" {sys_idx}"), Style::default().fg(Color::DarkGray)),
    ];

    if !repo_count.is_empty() {
        spans.push(Span::styled(
            format!(" | {repo_count}"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    spans.extend([
        Span::raw(" | "),
        Span::styled(&status_detail, Style::default().fg(Color::DarkGray)),
        Span::raw(" | "),
        Span::styled(
            "q:quit Tab:sys Enter:load /:filter a/p/t:actions A:bulk",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let status = Line::from(spans);
    let widget = Paragraph::new(status).block(Block::default().borders(Borders::TOP));

    f.render_widget(widget, area);
}
