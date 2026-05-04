use anyhow::Result;
use tokio::sync::mpsc;

use crate::cache::{self, CachedRepoEntry, DiskCache};
use crate::config::manifest::BranchProtectionConfig;
use crate::config::templates::load_templates_with_custom_dir;
use crate::config::{Manifest, SecurityConfig};
use crate::detection::project_type::ProjectType;
use crate::github::Client;
use crate::github::commits::CommitFile;

use super::state::{BgMessage, RepoEntry};

pub(super) fn spawn_repo_load(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    manifest: &Manifest,
    system_id: &str,
    num_custom_checks: usize,
) {
    let tx = tx.clone();
    let bg_client = client.clone();
    let excludes = manifest.exclude_patterns_for_system(system_id);
    let explicit = manifest.explicit_repos_for_system(system_id);
    let sys_id = system_id.to_owned();

    tokio::spawn(async move {
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
                        dependency_graph: None,
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

pub(super) fn save_to_disk_cache(
    disk_cache: &Option<DiskCache>,
    system_id: &str,
    repos: &[RepoEntry],
) {
    let entries: Vec<CachedRepoEntry> = repos
        .iter()
        .map(|re| CachedRepoEntry {
            repo: re.repo.clone(),
            security: re.security.clone(),
            dependency_graph: re.dependency_graph.clone(),
        })
        .collect();
    cache::try_save(disk_cache, system_id, &entries);
}

pub(super) fn spawn_security_load(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repos: &[RepoEntry],
) {
    for (idx, entry) in repos.iter().enumerate() {
        let tx = tx.clone();
        let bg_client = client.clone();
        let repo_name = entry.repo.name.clone();
        let repo_data = entry
            .repo
            .security_and_analysis
            .as_ref()
            .map(|sa| serde_json::json!({ "security_and_analysis": sa }));

        tokio::spawn(async move {
            let result = bg_client
                .get_security_state_with_repo_data(&repo_name, repo_data.as_ref())
                .await;
            let dependency_graph = bg_client.audit_dependency_graph(&repo_name).await;
            if let Ok(state) = result {
                let _ = tx.send(BgMessage::SecurityLoaded(idx, state, dependency_graph));
            }
        });
    }
}

pub(super) fn spawn_custom_checks(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repos: &[RepoEntry],
    checks: &[crate::config::SecurityCheck],
) {
    if checks.is_empty() {
        return;
    }

    for (repo_idx, entry) in repos.iter().enumerate() {
        for (check_idx, check) in checks.iter().enumerate() {
            let tx = tx.clone();
            let repo = entry.repo.clone();
            let check = check.clone();
            let bg_client = client.clone();

            tokio::spawn(async move {
                let result = bg_client.run_custom_check(&repo, &check).await;
                let _ = tx.send(BgMessage::CustomCheckLoaded(repo_idx, check_idx, result));
            });
        }
    }
}

pub(super) fn spawn_security_apply(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repo_name: &str,
    config: &SecurityConfig,
) {
    let tx = tx.clone();
    let bg_client = client.clone();
    let repo = repo_name.to_owned();
    let cfg = config.clone();

    tokio::spawn(async move {
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

pub(super) fn spawn_protection_apply(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    repo: &str,
    branch: &str,
    config: &BranchProtectionConfig,
) {
    let tx = tx.clone();
    let bg_client = client.clone();
    let repo = repo.to_owned();
    let branch = branch.to_owned();
    let cfg = config.clone();

    tokio::spawn(async move {
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

pub(super) fn spawn_template_deploy(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    manifest: &Manifest,
    repo_name: &str,
    default_branch: &str,
    template_name: &str,
) {
    let tx = tx.clone();
    let bg_client = client.clone();
    let repo = repo_name.to_owned();
    let default_br = default_branch.to_owned();
    let template = template_name.to_owned();
    let branch_name = manifest.templates.branch.clone();
    let reviewers = manifest.templates.reviewers.clone();
    let commit_prefix = manifest.templates.commit_message_prefix.clone();
    let custom_dir = manifest.templates.custom_dir.clone();
    let registries = manifest.templates.registries.clone();

    tokio::spawn(async move {
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

pub(super) fn spawn_settings_apply(
    tx: &mpsc::UnboundedSender<BgMessage>,
    client: &Client,
    manifest: &Manifest,
    repo_name: &str,
    default_branch: &str,
) {
    let tx = tx.clone();
    let bg_client = client.clone();
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
        let result = async {
            let mut actions: Vec<String> = Vec::new();

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

pub(super) async fn detect_project_type_bg(client: &Client, repo: &str) -> Result<ProjectType> {
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

pub(super) async fn detect_java_version_bg(client: &Client, repo: &str) -> Result<u8> {
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

pub(super) async fn detect_node_version_bg(client: &Client, repo: &str) -> Result<String> {
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
