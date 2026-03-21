use anyhow::{Context, Result};
use clap::Args;
use console::style;
use dialoguer::Confirm;

use crate::config::Manifest;
use crate::config::templates::load_templates_with_custom_dir;
use crate::detection::project_type::ProjectType;
use crate::detection::versions;
use crate::engine::audit_log::AuditLog;
use crate::github::Client;
use crate::github::commits::CommitFile;

#[derive(Args)]
pub struct CommitCommand {
    #[command(subcommand)]
    action: CommitAction,
}

#[derive(clap::Subcommand)]
enum CommitAction {
    /// Preview what files would be committed
    Plan {
        /// Template name (dependabot, codeql, dependency-submission)
        #[arg(long)]
        template: String,
    },

    /// Commit template files and create PRs
    Apply {
        /// Template name (dependabot, codeql, dependency-submission)
        #[arg(long)]
        template: String,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

/// Resolved template output for a single repo.
struct TemplateResult {
    repo_name: String,
    target_path: String,
    rendered: String,
    already_exists: bool,
    existing_matches: bool,
}

impl CommitCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
        repo: Option<&str>,
    ) -> Result<()> {
        match &self.action {
            CommitAction::Plan { template } => plan(client, manifest, system, repo, template).await,
            CommitAction::Apply { template, yes } => {
                apply(client, manifest, system, repo, template, *yes).await
            }
        }
    }
}

fn resolve_template_info(template: &str) -> Result<(&str, &str)> {
    match template {
        "dependabot" => Ok((".github/dependabot.yml", "dependabot")),
        "codeql" => Ok((".github/workflows/codeql.yml", "codeql")),
        "dependency-submission" => Ok((
            ".github/workflows/dependency-submission.yml",
            "dependency-submission",
        )),
        _ => anyhow::bail!(
            "Unknown template: {template}. Available: dependabot, codeql, dependency-submission"
        ),
    }
}

async fn detect_and_render(
    client: &Client,
    repo_name: &str,
    default_branch: &str,
    template_category: &str,
    target_path: &str,
    manifest: &Manifest,
) -> Result<TemplateResult> {
    // Detect project type by checking for build files
    let project_type = detect_project_type(client, repo_name).await?;

    let tera_template_name = match (&project_type, template_category) {
        (ProjectType::Gradle, "dependabot") => "dependabot/gradle.yml.tera",
        (ProjectType::Npm, "dependabot") => "dependabot/npm.yml.tera",
        (ProjectType::Gradle, "codeql") => "codeql/gradle.yml.tera",
        (ProjectType::Npm, "codeql") => "codeql/npm.yml.tera",
        (ProjectType::Gradle, "dependency-submission") => "dependency-submission/gradle.yml.tera",
        (pt, cat) => {
            anyhow::bail!("No template for {cat} + {pt} in repo {repo_name}");
        }
    };

    // Detect version
    let mut tera_context = tera::Context::new();
    tera_context.insert("default_branch", default_branch);

    match project_type {
        ProjectType::Gradle => {
            let java_ver = detect_java_version(client, repo_name).await?;
            tera_context.insert("java_version", &java_ver.to_string());

            if let Some(reg) = manifest.templates.registries.get("gradle-artifactory") {
                tera_context.insert("registry_url", &reg.url);
                if let Some(ref provider) = reg.jfrog_oidc_provider {
                    tera_context.insert("jfrog_oidc_provider", provider);
                }
            }
        }
        ProjectType::Npm => {
            let node_ver = detect_node_version(client, repo_name).await?;
            tera_context.insert("node_version", &node_ver);
        }
        _ => {}
    }

    let tera = load_templates_with_custom_dir(
        manifest
            .templates
            .custom_dir
            .as_ref()
            .map(std::path::Path::new),
    )?;
    let rendered = tera
        .render(tera_template_name, &tera_context)
        .with_context(|| format!("Failed to render template {tera_template_name}"))?;

    // Check if file already exists
    let existing = client.get_file(repo_name, target_path, None).await?;
    let (already_exists, existing_matches) = if let Some(ref content) = existing {
        let decoded = Client::decode_content(content).unwrap_or_default();
        (true, decoded.trim() == rendered.trim())
    } else {
        (false, false)
    };

    Ok(TemplateResult {
        repo_name: repo_name.to_owned(),
        target_path: target_path.to_owned(),
        rendered,
        already_exists,
        existing_matches,
    })
}

async fn detect_project_type(client: &Client, repo: &str) -> Result<ProjectType> {
    // Check for Gradle first (more common in our org)
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

async fn detect_java_version(client: &Client, repo: &str) -> Result<u8> {
    // Try build.gradle.kts first, then build.gradle
    for file in &["build.gradle.kts", "build.gradle"] {
        if let Some(content) = client.get_file(repo, file, None).await? {
            let text = Client::decode_content(&content)?;
            if let Some(ver) = versions::extract_java_version(&text) {
                tracing::info!("{repo}: detected Java {ver} from {file}");
                return Ok(ver);
            }
        }
    }

    tracing::warn!("{repo}: could not detect Java version, defaulting to 21");
    Ok(21)
}

async fn detect_node_version(client: &Client, repo: &str) -> Result<String> {
    if let Some(content) = client.get_file(repo, "package.json", None).await? {
        let text = Client::decode_content(&content)?;
        if let Some(ver) = versions::extract_node_version(&text) {
            // Extract just the major version number
            let major: String = ver.chars().filter(|c| c.is_ascii_digit()).collect();
            if !major.is_empty() {
                tracing::info!("{repo}: detected Node {major} from package.json");
                return Ok(major);
            }
        }
    }

    tracing::warn!("{repo}: could not detect Node version, defaulting to 20");
    Ok("20".to_owned())
}

async fn resolve_repos_with_branches(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
) -> Result<Vec<(String, String)>> {
    if let Some(repo_name) = repo {
        let r = client.get_repo(repo_name).await?;
        return Ok(vec![(r.name, r.default_branch)]);
    }

    let sys = system.ok_or_else(|| anyhow::anyhow!("Either --system or --repo is required"))?;
    let excludes = manifest.exclude_patterns_for_system(sys);
    let repos = client.list_repos_for_system(sys, &excludes).await?;
    Ok(repos
        .into_iter()
        .map(|r| (r.name, r.default_branch))
        .collect())
}

async fn plan(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    template: &str,
) -> Result<()> {
    let (target_path, template_category) = resolve_template_info(template)?;
    let repos = resolve_repos_with_branches(client, manifest, system, repo).await?;

    println!();
    println!(
        "  {} Commit plan: {} → {}",
        style("📋").bold(),
        style(template).cyan().bold(),
        style(target_path).dim()
    );
    println!(
        "  {} Scanning {} repositories...",
        style("🔍").bold(),
        repos.len()
    );
    println!();

    let mut to_create = 0;
    let mut to_update = 0;
    let mut up_to_date = 0;
    let mut skipped = 0;

    for (repo_name, default_branch) in &repos {
        match detect_and_render(
            client,
            repo_name,
            default_branch,
            template_category,
            target_path,
            manifest,
        )
        .await
        {
            Ok(result) => {
                if result.existing_matches {
                    println!(
                        "  {} {}",
                        style("✓").green(),
                        style(&result.repo_name).dim()
                    );
                    up_to_date += 1;
                } else if result.already_exists {
                    println!(
                        "  {} {} (update {})",
                        style("⚡").yellow(),
                        style(&result.repo_name).bold(),
                        target_path
                    );
                    to_update += 1;
                } else {
                    println!(
                        "  {} {} (create {})",
                        style("⚡").yellow(),
                        style(&result.repo_name).bold(),
                        target_path
                    );
                    to_create += 1;
                }
            }
            Err(e) => {
                println!(
                    "  {} {}: {}",
                    style("⏭").dim(),
                    style(&repo_name).dim(),
                    style(e).dim()
                );
                skipped += 1;
            }
        }
    }

    println!();
    println!(
        "  Summary: {} to create, {} to update, {} up to date, {} skipped",
        style(to_create).yellow().bold(),
        style(to_update).yellow().bold(),
        style(up_to_date).green(),
        style(skipped).dim()
    );

    if to_create + to_update > 0 {
        println!(
            "\n  Run {} to apply.",
            style(format!("ward commit apply --template {template}"))
                .cyan()
                .bold()
        );
    }

    Ok(())
}

async fn apply(
    client: &Client,
    manifest: &Manifest,
    system: Option<&str>,
    repo: Option<&str>,
    template: &str,
    yes: bool,
) -> Result<()> {
    let (target_path, template_category) = resolve_template_info(template)?;
    let repos = resolve_repos_with_branches(client, manifest, system, repo).await?;
    let branch_name = &manifest.templates.branch;

    println!();
    println!(
        "  {} Preparing commits: {} → {}",
        style("📋").bold(),
        style(template).cyan().bold(),
        style(target_path).dim()
    );

    // Build all template results
    let mut pending: Vec<TemplateResult> = Vec::new();
    for (repo_name, default_branch) in &repos {
        match detect_and_render(
            client,
            repo_name,
            default_branch,
            template_category,
            target_path,
            manifest,
        )
        .await
        {
            Ok(result) if !result.existing_matches => {
                pending.push(result);
            }
            Ok(_) => {
                tracing::debug!("{repo_name}: already up to date, skipping");
            }
            Err(e) => {
                tracing::warn!("{repo_name}: skipped ({e})");
            }
        }
    }

    if pending.is_empty() {
        println!(
            "\n  {} All repositories already up to date.",
            style("✅").green()
        );
        return Ok(());
    }

    println!(
        "\n  {} repos need changes. Branch: {}",
        style(pending.len()).yellow().bold(),
        style(branch_name).cyan()
    );

    for r in &pending {
        let action = if r.already_exists { "update" } else { "create" };
        println!(
            "  {} {} → {action} {}",
            style("⚡").yellow(),
            r.repo_name,
            r.target_path
        );
    }

    if !yes {
        println!();
        let proceed = Confirm::new()
            .with_prompt(format!(
                "  Commit to {} repos and create PRs?",
                pending.len()
            ))
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

    for result in &pending {
        println!("  {} {} ...", style("▶").magenta(), result.repo_name);

        let default_branch = repos
            .iter()
            .find(|(n, _)| *n == result.repo_name)
            .map(|(_, b)| b.as_str())
            .unwrap_or("main");

        match commit_and_pr(&CommitPrParams {
            client,
            repo: &result.repo_name,
            default_branch,
            branch_name,
            target_path: &result.target_path,
            content: &result.rendered,
            template,
            reviewers: &manifest.templates.reviewers,
            commit_prefix: &manifest.templates.commit_message_prefix,
        })
        .await
        {
            Ok(pr_url) => {
                println!("    {} PR: {}", style("✅").green(), style(&pr_url).cyan());
                audit_log.log(
                    &result.repo_name,
                    &format!("commit_template_{template}"),
                    "success",
                    result.already_exists,
                    true,
                )?;
                succeeded += 1;
            }
            Err(e) => {
                println!("    {} {}", style("❌").red(), e);
                failed.push((result.repo_name.clone(), e.to_string()));
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!(
            "  {} All {} repos committed and PRs created.",
            style("✅").green(),
            succeeded
        );
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

struct CommitPrParams<'a> {
    client: &'a Client,
    repo: &'a str,
    default_branch: &'a str,
    branch_name: &'a str,
    target_path: &'a str,
    content: &'a str,
    template: &'a str,
    reviewers: &'a [String],
    commit_prefix: &'a str,
}

async fn commit_and_pr(params: &CommitPrParams<'_>) -> Result<String> {
    let CommitPrParams {
        client,
        repo,
        default_branch,
        branch_name,
        target_path,
        content,
        template,
        reviewers,
        commit_prefix,
    } = params;

    // Create branch from default branch
    client
        .create_branch(repo, branch_name, default_branch)
        .await?;

    // Commit the file
    let message = format!("{commit_prefix}add {template} configuration");
    let files = vec![CommitFile {
        path: target_path.to_string(),
        content: content.to_string(),
    }];

    client
        .create_commit(repo, branch_name, &message, &files)
        .await?;

    // Create PR
    let pr_title = format!("{commit_prefix}add {template} configuration");
    let pr_body = format!(
        "## Ward: automated template commit\n\n\
         Template: `{template}`\n\
         File: `{target_path}`\n\n\
         This PR was created by [ward](https://github.com/OriginalMHV/ward).\n\n\
         ---\n\
         *Review the file contents, then merge.*"
    );

    let pr = client
        .create_pull_request(
            repo,
            &pr_title,
            &pr_body,
            branch_name,
            default_branch,
            reviewers,
        )
        .await?;

    Ok(pr.html_url)
}
