use anyhow::Result;
use clap::Args;
use console::style;

use crate::config::Manifest;
use crate::github::Client;

#[derive(Args)]
pub struct AuditCommand {
    /// Output format
    #[arg(long, default_value = "table")]
    format: String,
}

impl AuditCommand {
    pub async fn run(
        &self,
        client: &Client,
        manifest: &Manifest,
        system: Option<&str>,
    ) -> Result<()> {
        // Re-use the security audit for now; Phase 4 will expand this significantly
        let sys = system.ok_or_else(|| {
            anyhow::anyhow!("--system is required for audit")
        })?;

        let excludes = manifest.exclude_patterns_for_system(sys);
        let repos = client.list_repos_for_system(sys, &excludes).await?;
        let repo_names: Vec<String> = repos.into_iter().map(|r| r.name).collect();

        println!();
        println!(
            "  {} Auditing {} repos in system {}...",
            style("🔍").bold(),
            repo_names.len(),
            style(sys).cyan()
        );

        if self.format == "json" {
            let mut results = Vec::new();
            for repo_name in &repo_names {
                let state = client.get_security_state(repo_name).await?;
                results.push(serde_json::json!({
                    "repo": repo_name,
                    "security": state,
                }));
            }
            println!("{}", serde_json::to_string_pretty(&results)?);
        } else {
            // Table format — delegate to security audit
            println!(
                "  {} Use `ward security audit --system {sys}` for detailed security audit.",
                style("ℹ").blue()
            );
            println!(
                "  {} Full audit dashboard coming in Phase 4.",
                style("🚧").yellow()
            );
        }

        Ok(())
    }
}
