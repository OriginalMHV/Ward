use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use console::style;

use crate::config::Manifest;
use crate::config::manifest::CategoryPolicy;

#[derive(clap::Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(clap::Subcommand)]
pub enum ConfigAction {
    /// Display current configuration
    Show,
    /// Open configuration in editor
    Edit,
    /// Show configuration file path
    Path,
    /// Removed. Edit ward.toml directly or run `ward config edit`
    #[command(hide = true)]
    Set {
        #[arg(num_args = 0.., allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Removed. Edit ward.toml directly or run `ward config edit`
    #[command(hide = true)]
    AddSystem {
        #[arg(num_args = 0.., allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Removed. Edit ward.toml directly or run `ward config edit`
    #[command(hide = true)]
    RemoveSystem {
        #[arg(num_args = 0.., allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

impl ConfigCommand {
    pub fn run(self, config_override: Option<&str>) -> Result<()> {
        match self.action {
            ConfigAction::Show => run_show(config_override),
            ConfigAction::Edit => run_edit(config_override),
            ConfigAction::Path => {
                run_path(config_override);
                Ok(())
            }
            ConfigAction::Set { .. } => removed_subcommand("set"),
            ConfigAction::AddSystem { .. } => removed_subcommand("add-system"),
            ConfigAction::RemoveSystem { .. } => removed_subcommand("remove-system"),
        }
    }
}

fn removed_subcommand(name: &str) -> Result<()> {
    bail!("`ward config {name}` was removed. Edit ward.toml directly or run `ward config edit`.")
}

pub fn resolve_config_path(config_override: Option<&str>) -> PathBuf {
    match config_override {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("ward.toml"),
    }
}

fn run_show(config_override: Option<&str>) -> Result<()> {
    let path = resolve_config_path(config_override);
    if !path.exists() {
        println!(
            "  {} No configuration file found at {}",
            style("[!!]").yellow(),
            path.display()
        );
        println!("  Run {} to create one.", style("ward init").bold());
        return Ok(());
    }

    let manifest = Manifest::load(config_override)?;

    println!();
    println!("  {}", style("Ward manifest").bold());
    println!("  {}", style("Organization").bold());
    println!("    name: {}", style(&manifest.org.name).cyan());

    println!();
    println!("  {}", style("Categories").bold());
    let categories = &manifest.categories;
    let mut category_count = 0;
    if let Some(category) = &categories.repository {
        print_category(
            "repository",
            &category.policy,
            "repository settings and metadata",
        );
        category_count += 1;
    }
    if let Some(category) = &categories.security {
        let configured = [
            category.advanced_security,
            category.code_security,
            category.dependabot_alerts,
            category.dependabot_security_updates,
            category.secret_scanning,
            category.secret_scanning_push_protection,
            category.secret_scanning_validity_checks,
            category.secret_scanning_non_provider_patterns,
            category.secret_scanning_ai_detection,
            category.private_vulnerability_reporting,
        ]
        .into_iter()
        .flatten()
        .count();
        print_category(
            "security",
            &category.policy,
            &format!("{configured} configured setting(s)"),
        );
        category_count += 1;
    }
    if let Some(category) = &categories.branch_protection {
        let summary = match (
            category.default_branch.is_some(),
            category.default_branch_detailed.is_some(),
            category.protected_branches.len(),
        ) {
            (_, true, count) if count > 0 => {
                format!("detailed default branch and {count} protected branch(es)")
            }
            (_, true, _) => "detailed default branch".to_owned(),
            (true, _, count) if count > 0 => {
                format!("default branch and {count} protected branch(es)")
            }
            (true, _, _) => "default branch".to_owned(),
            (false, _, count) => format!("{count} protected branch(es)"),
        };
        print_category("branch_protection", &category.policy, &summary);
        category_count += 1;
    }
    if let Some(category) = &categories.rulesets {
        print_category(
            "rulesets",
            &category.policy,
            &format!(
                "{} repository ruleset(s)",
                category.repository_rulesets.len()
            ),
        );
        category_count += 1;
    }
    if let Some(category) = &categories.files {
        print_category(
            "files",
            &category.policy,
            &format!("{} managed file(s)", category.entries.len()),
        );
        category_count += 1;
    }
    if let Some(category) = &categories.actions {
        print_category("actions", &category.policy, "Actions configuration");
        category_count += 1;
    }
    if let Some(category) = &categories.environments {
        print_category(
            "environments",
            &category.policy,
            &format!("{} environment(s)", category.entries.len()),
        );
        category_count += 1;
    }
    if let Some(category) = &categories.access {
        print_category(
            "access",
            &category.policy,
            &format!(
                "{} team(s), {} collaborator(s)",
                category.teams.len(),
                category.collaborators.len()
            ),
        );
        category_count += 1;
    }
    if let Some(category) = &categories.integrations {
        print_category("integrations", &category.policy, "repository integrations");
        category_count += 1;
    }
    if category_count == 0 {
        println!("    (no categories configured)");
    }

    println!();
    println!("  {}", style("File delivery").bold());
    println!("    branch: {}", manifest.file_delivery.branch);
    println!(
        "    commit_message_prefix: {}",
        manifest.file_delivery.commit_message_prefix
    );

    if manifest.systems.is_empty() {
        println!();
        println!("  {}", style("Systems").bold());
        println!("    (none)");
    } else {
        for sys in &manifest.systems {
            println!();
            println!("  {}", style(format!("System: {}", sys.name)).bold());
            println!("    id: {}", style(&sys.id).cyan());
            if !sys.exclude.is_empty() {
                println!("    exclude: {}", sys.exclude.join(", "));
            }
            if !sys.repos.is_empty() {
                println!("    repos: {}", sys.repos.join(", "));
            }
        }
    }

    println!();
    Ok(())
}

fn print_category(name: &str, policy: &CategoryPolicy, summary: &str) {
    let disposition = format!("{:?}", policy.disposition).to_lowercase();
    let sensitive = if policy.sensitive { ", sensitive" } else { "" };
    println!(
        "    {}: {}{} — {}",
        style(name).cyan(),
        disposition,
        sensitive,
        summary
    );
}

fn run_edit(config_override: Option<&str>) -> Result<()> {
    let path = resolve_config_path(config_override);
    if !path.exists() {
        bail!(
            "No configuration file at {}. Run `ward init` first.",
            path.display()
        );
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "the editor choice comes from the user's environment"
    )]
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_owned());

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to open editor '{editor}'"))?;

    if !status.success() {
        bail!("Editor exited with non-zero status");
    }

    let content = std::fs::read_to_string(&path)?;
    match toml::from_str::<Manifest>(&content) {
        Ok(_) => println!("  {} Configuration is valid.", style("[ok]").green()),
        Err(e) => {
            println!("  {} Configuration has errors: {e}", style("[!!]").red());
        }
    }

    Ok(())
}

fn run_path(config_override: Option<&str>) {
    let path = resolve_config_path(config_override);
    let abs = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    println!("{}", abs.display());
    if path.exists() {
        println!("  {} File exists.", style("[ok]").green());
    } else {
        println!("  {} File does not exist.", style("[..]").yellow());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_config_path_default() {
        let path = resolve_config_path(None);
        assert_eq!(path, PathBuf::from("ward.toml"));
    }

    #[test]
    fn test_resolve_config_path_override() {
        let path = resolve_config_path(Some("/tmp/custom.toml"));
        assert_eq!(path, PathBuf::from("/tmp/custom.toml"));
    }

    #[test]
    fn removed_subcommands_fail_and_name_the_replacement() {
        for name in ["set", "add-system", "remove-system"] {
            let message = removed_subcommand(name).unwrap_err().to_string();
            assert!(message.contains(&format!("ward config {name}")));
            assert!(message.contains("ward config edit"));
        }
    }
}
