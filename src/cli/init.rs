use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use console::style;

use crate::cli::import::{ImportOptions, import_repository};

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

    /// Accepted for compatibility. Init always writes the minimal scaffold.
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

        write_default()
    }
}

/// Write `content` to `path` only if the file does not exist.
/// Returns `false` when it already exists. The check and the write are one atomic step.
fn create_new_file(path: &std::path::Path, content: &str) -> Result<bool> {
    use std::io::Write;

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(content.as_bytes())
                .with_context(|| format!("Failed to write {}", path.display()))?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).with_context(|| format!("Failed to create {}", path.display())),
    }
}

fn write_default() -> Result<()> {
    if !create_new_file(std::path::Path::new(OUTPUT_PATH), EXAMPLE_MANIFEST)? {
        println!("  {} ward.toml already exists.", style("warning").yellow());
        return Ok(());
    }

    println!(
        "  {} Created ward.toml - edit it to configure your org and systems.",
        style("ok").green()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Manifest;
    use crate::config::manifest::ManagementDisposition;

    #[test]
    fn non_interactive_flag_is_still_accepted() {
        use clap::Parser;

        let cli = crate::cli::Cli::parse_from(["ward", "init", "--non-interactive"]);
        assert!(matches!(cli.command, crate::cli::Command::Init(_)));
    }

    #[test]
    fn create_new_file_never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ward.toml");

        assert!(create_new_file(&path, "first").unwrap());
        assert!(!create_new_file(&path, "second").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
    }

    #[cfg(unix)]
    #[test]
    fn create_new_file_does_not_follow_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("ward.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(!create_new_file(&link, "content").unwrap());
        assert!(!target.exists());
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
}
