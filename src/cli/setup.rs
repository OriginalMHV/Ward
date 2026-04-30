use anyhow::{Context, Result};
use clap::Args;
use console::style;
use dialoguer::{Confirm, Input, MultiSelect};

use crate::config::Manifest;
use crate::engine::{audit_log::AuditLog, executor, planner, verifier};
use crate::github::Client;

#[derive(Args)]
pub struct SetupCommand {
    /// Repository name to set up (optional — prompted if not given)
    pub repo: Option<String>,

    /// Path to ward.toml (created if missing)
    #[arg(long)]
    pub config: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Feature {
    Security,
    Rulesets,
    Templates,
}

impl std::fmt::Display for Feature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Feature::Security => {
                write!(f, "Security (Dependabot, secret scanning, push protection)")
            }
            Feature::Rulesets => write!(f, "Rulesets (branch protection, review requirements)"),
            Feature::Templates => write!(f, "Templates (Dependabot config, CI workflows)"),
        }
    }
}

/// Generate a ward.toml config string for the given organization.
/// Used by `ward setup` when no config file exists yet.
fn generate_config(org: &str) -> String {
    format!(
        r#"[org]
name = "{org}"

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

[rulesets.branch_protection]
enabled = true
required_approvals = 1
dismiss_stale_reviews = true
require_code_owner_reviews = true
block_force_pushes = true
"#
    )
}

impl SetupCommand {
    pub async fn run(&self) -> Result<()> {
        println!();
        println!("  {}", style("Ward Setup").bold());
        println!(
            "  {}",
            style(
                "Guided setup for a single repository. Nothing changes without your confirmation."
            )
            .dim()
        );
        println!();

        // Step 1: Ensure ward.toml exists
        let config_path = self.config.as_deref().unwrap_or("ward.toml");
        let manifest = self.ensure_config(config_path)?;

        // Step 2: Resolve token and create client
        let org = &manifest.org.name;
        println!(
            "  {} Organization: {}",
            style("▸").cyan(),
            style(org).bold()
        );

        let client = Client::new(org, 5).await?;

        // Step 3: Get repo name
        let repo_name = match &self.repo {
            Some(name) => name.clone(),
            None => Input::new()
                .with_prompt("  Repository name")
                .interact_text()
                .context("Failed to read repo name")?,
        };

        // Verify repo exists
        print!(
            "  {} Checking {}...",
            style("[..]").dim(),
            style(&repo_name).bold()
        );
        match client.get_repo(&repo_name).await {
            Ok(_) => {
                println!(
                    "\r  {} Repository found: {}         ",
                    style("✓").green(),
                    &repo_name
                );
            }
            Err(_) => {
                println!(
                    "\r  {} Repository not found: {}         ",
                    style("✗").red(),
                    &repo_name
                );
                println!();
                println!(
                    "  Make sure the repo exists in the {} organization.",
                    style(org).bold()
                );
                return Ok(());
            }
        }

        // Step 4: Choose features
        println!();
        let features = vec![Feature::Security, Feature::Rulesets, Feature::Templates];
        let defaults = vec![true, true, false];

        let selected = MultiSelect::new()
            .with_prompt("  Which features do you want to configure?")
            .items(&features)
            .defaults(&defaults)
            .interact()
            .context("Failed to get feature selection")?;

        if selected.is_empty() {
            println!();
            println!(
                "  {} No features selected. Nothing to do.",
                style("!").yellow()
            );
            return Ok(());
        }

        let chosen: Vec<Feature> = selected.into_iter().map(|i| features[i]).collect();

        // Step 5: Plan (always show what would happen first)
        println!();
        println!(
            "  {} {}",
            style("Planning").bold(),
            style("(read-only — no changes yet)").dim()
        );
        println!();

        let mut security_plans: Vec<planner::RepoPlan> = Vec::new();

        if chosen.contains(&Feature::Security) {
            let desired = manifest.security_for_system("default");
            let current = client.get_security_state(&repo_name).await?;
            let plan = planner::plan_security(&repo_name, &current, desired);

            if plan.has_changes() {
                println!(
                    "  {} Security changes for {}:",
                    style("→").cyan(),
                    style(&repo_name).bold()
                );
                for change in &plan.changes {
                    println!(
                        "      {} {} (currently: {}, desired: {})",
                        style("→").yellow(),
                        change.feature,
                        if change.current { "on" } else { "off" },
                        if change.desired { "on" } else { "off" },
                    );
                }
                security_plans.push(plan);
            } else {
                println!("  {} Security: already compliant ✓", style("✓").green());
            }
        }

        if chosen.contains(&Feature::Rulesets) {
            if manifest.rulesets.branch_protection.is_some() {
                println!(
                    "  {} Rulesets: configured in ward.toml — run {} to preview",
                    style("ℹ").blue(),
                    style(format!("ward rulesets plan --repo {}", &repo_name)).cyan()
                );
            } else {
                println!(
                    "  {} Rulesets: not configured in ward.toml yet",
                    style("!").yellow()
                );
                println!(
                    "      Add a {} section to ward.toml to enable.",
                    style("[rulesets.branch_protection]").bold()
                );
            }
        }

        if chosen.contains(&Feature::Templates) {
            println!(
                "  {} Templates: run {} to see available templates",
                style("ℹ").blue(),
                style("ward template list").cyan()
            );
            println!(
                "      Then: {}",
                style(format!(
                    "ward commit plan --repo {} --template <name>",
                    &repo_name
                ))
                .cyan()
            );
        }

        // Step 6: Apply security (only with explicit confirmation)
        if !security_plans.is_empty() {
            println!();
            println!(
                "  {}",
                style("The above changes will enable security features on your repository.").dim()
            );

            let apply = Confirm::new()
                .with_prompt("  Apply security changes now?")
                .default(true)
                .interact()
                .unwrap_or(false);

            if apply {
                println!();
                println!("  {} Applying...", style("[>>]").bold());

                let audit_log = AuditLog::new()?;
                let report =
                    executor::execute_security_plan(&client, &security_plans, &audit_log).await?;
                report.print_summary();

                // Verify
                println!();
                println!("  {} Verifying...", style("[..]").bold());
                let desired = manifest.security_for_system("default");
                let verify_report =
                    verifier::verify_security(&client, &security_plans, desired).await?;
                verify_report.print_summary();
            } else {
                println!();
                println!(
                    "  No changes made. Run {} when ready.",
                    style(format!("ward security apply --repo {}", &repo_name)).cyan()
                );
            }
        }

        // Step 7: Summary & next steps
        println!();
        println!("  {}", style("─────────────────────────────────").dim());
        println!("  {}", style("What to do next:").bold());
        println!();

        if chosen.contains(&Feature::Rulesets) && manifest.rulesets.branch_protection.is_some() {
            println!(
                "    {} ward rulesets plan --repo {}     (preview branch rules)",
                style("1.").cyan(),
                &repo_name
            );
            println!(
                "    {} ward rulesets apply --repo {}    (apply when happy)",
                style("2.").cyan(),
                &repo_name
            );
        }

        if chosen.contains(&Feature::Templates) {
            println!(
                "    {} ward template list                       (see available templates)",
                style("3.").cyan(),
            );
            println!(
                "    {} ward commit plan --repo {} --template X  (preview file commit)",
                style("4.").cyan(),
                &repo_name
            );
        }

        println!();
        println!(
            "  {} Always use {} first to see what would change.",
            style("tip").dim(),
            style("plan").bold()
        );
        println!("  {}", style("  Full guide: docs/getting-started.md").dim());
        println!();

        Ok(())
    }

    fn ensure_config(&self, config_path: &str) -> Result<Manifest> {
        if std::path::Path::new(config_path).exists() {
            println!(
                "  {} Using config: {}",
                style("✓").green(),
                style(config_path).underlined()
            );
            return Manifest::load(Some(config_path));
        }

        println!(
            "  {} No ward.toml found at {}",
            style("!").yellow(),
            config_path
        );
        println!();

        let create = Confirm::new()
            .with_prompt("  Create a ward.toml now?")
            .default(true)
            .interact()
            .unwrap_or(false);

        if !create {
            anyhow::bail!("ward.toml is required. Run `ward init` to create one.");
        }

        let org: String = Input::new()
            .with_prompt("  GitHub organization name")
            .interact_text()
            .context("Failed to read org name")?;

        let content = generate_config(&org);

        std::fs::write(config_path, &content)?;
        println!();
        println!(
            "  {} Created {}",
            style("✓").green(),
            style(config_path).underlined()
        );
        println!(
            "  {} Review and adjust settings as needed.",
            style("tip:").dim()
        );
        println!();

        Manifest::load(Some(config_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generate_config_produces_valid_toml() {
        let content = generate_config("acme-engineering");
        let manifest: Manifest = toml::from_str(&content).unwrap();

        assert_eq!(manifest.org.name, "acme-engineering");
    }

    #[test]
    fn generate_config_enables_all_security_features() {
        let content = generate_config("test-org");
        let manifest: Manifest = toml::from_str(&content).unwrap();

        assert!(manifest.security.secret_scanning);
        assert!(manifest.security.secret_scanning_ai_detection);
        assert!(manifest.security.push_protection);
        assert!(manifest.security.dependabot_alerts);
        assert!(manifest.security.dependabot_security_updates);
    }

    #[test]
    fn generate_config_includes_rulesets() {
        let content = generate_config("test-org");
        let manifest: Manifest = toml::from_str(&content).unwrap();

        let rulesets = manifest.rulesets.branch_protection.unwrap();
        assert!(rulesets.enabled);
        assert_eq!(rulesets.required_approvals, 1);
        assert!(rulesets.dismiss_stale_reviews);
        assert!(rulesets.require_code_owner_reviews);
        assert!(rulesets.block_force_pushes);
    }

    #[test]
    fn generate_config_includes_template_settings() {
        let content = generate_config("test-org");
        let manifest: Manifest = toml::from_str(&content).unwrap();

        assert_eq!(manifest.templates.branch, "chore/ward-setup");
        assert_eq!(manifest.templates.commit_message_prefix, "chore: ");
    }

    #[test]
    fn generate_config_handles_org_with_hyphens_and_numbers() {
        let content = generate_config("my-org-123");
        let manifest: Manifest = toml::from_str(&content).unwrap();
        assert_eq!(manifest.org.name, "my-org-123");
    }

    #[test]
    fn generate_config_file_is_writable_and_loadable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ward.toml");
        let content = generate_config("file-test-org");

        std::fs::write(&path, &content).unwrap();
        let manifest = Manifest::load(Some(path.to_str().unwrap())).unwrap();

        assert_eq!(manifest.org.name, "file-test-org");
        assert!(manifest.security.secret_scanning);
        assert!(manifest.rulesets.branch_protection.is_some());
    }

    #[test]
    fn feature_display_names_are_descriptive() {
        let security = format!("{}", Feature::Security);
        let rulesets = format!("{}", Feature::Rulesets);
        let templates = format!("{}", Feature::Templates);

        assert!(security.contains("Dependabot"));
        assert!(security.contains("secret scanning"));
        assert!(rulesets.contains("branch protection"));
        assert!(templates.contains("CI workflows"));
    }

    #[test]
    fn cli_parses_setup_with_repo_arg() {
        use clap::Parser;

        let cli = crate::cli::Cli::parse_from(["ward", "setup", "my-repo"]);
        match cli.command {
            crate::cli::Command::Setup(cmd) => {
                assert_eq!(cmd.repo, Some("my-repo".to_string()));
                assert_eq!(cmd.config, None);
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn cli_parses_setup_with_config_flag() {
        use clap::Parser;

        let cli = crate::cli::Cli::parse_from(["ward", "setup", "--config", "/tmp/my.toml"]);
        match cli.command {
            crate::cli::Command::Setup(cmd) => {
                assert_eq!(cmd.repo, None);
                assert_eq!(cmd.config, Some("/tmp/my.toml".to_string()));
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn cli_parses_setup_with_repo_and_config() {
        use clap::Parser;

        let cli =
            crate::cli::Cli::parse_from(["ward", "setup", "my-service", "--config", "custom.toml"]);
        match cli.command {
            crate::cli::Command::Setup(cmd) => {
                assert_eq!(cmd.repo, Some("my-service".to_string()));
                assert_eq!(cmd.config, Some("custom.toml".to_string()));
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn cli_parses_setup_no_args() {
        use clap::Parser;

        let cli = crate::cli::Cli::parse_from(["ward", "setup"]);
        match cli.command {
            crate::cli::Command::Setup(cmd) => {
                assert_eq!(cmd.repo, None);
                assert_eq!(cmd.config, None);
            }
            _ => panic!("Expected Setup command"),
        }
    }
}
