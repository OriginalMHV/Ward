use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use console::style;

use crate::cli::import::{ImportOptions, import_repository};

const EXAMPLE_MANIFEST: &str = r#"[org]
name = "your-github-org"

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
    /// Accepted for compatibility. Init never prompts.
    #[arg(long)]
    non_interactive: bool,

    /// Deprecated alias of `ward import`
    #[arg(long, hide = true, conflicts_with = "non_interactive")]
    from: Option<String>,

    #[arg(long, default_value = OUTPUT_PATH, requires = "from", hide = true)]
    output: PathBuf,

    #[arg(long, requires = "from", hide = true)]
    stdout: bool,

    #[arg(long, requires = "from", hide = true)]
    force: bool,

    #[arg(long, value_name = "OWNER/REPO", requires = "from", hide = true)]
    target: Vec<String>,

    #[arg(long, value_name = "GLOB", requires = "from", hide = true)]
    include: Vec<String>,

    #[arg(long, value_name = "GLOB", requires = "from", hide = true)]
    exclude: Vec<String>,

    #[arg(long, requires = "from", hide = true)]
    strict: bool,
}

impl InitCommand {
    /// The `--from` source of the deprecated delegate, if given.
    pub fn deprecated_source(&self) -> Option<&str> {
        self.from.as_deref()
    }

    /// The warning for `init --from`, without a trailing newline.
    pub fn deprecation_warning() -> &'static str {
        "warning: 'ward init --from' is deprecated and will be removed in 0.6.0; use 'ward import <SOURCE>'"
    }

    /// `parallelism` is the global `--parallelism` value.
    pub async fn run(&self, parallelism: usize) -> Result<()> {
        if let Some(source) = &self.from {
            eprintln!("{}", Self::deprecation_warning());
            return import_repository(ImportOptions {
                source,
                targets: &self.target,
                include: &self.include,
                exclude: &self.exclude,
                strict: self.strict,
                output: &self.output,
                stdout: self.stdout,
                force: self.force,
                parallelism,
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
    fn from_is_a_hidden_deprecated_delegate_with_import_options() {
        use clap::Parser;

        let cli = crate::cli::Cli::parse_from([
            "ward",
            "init",
            "--from",
            "acme/service",
            "--target",
            "other",
            "--stdout",
            "--strict",
        ]);
        let crate::cli::Command::Init(command) = cli.command else {
            panic!("expected init command");
        };
        assert_eq!(command.deprecated_source(), Some("acme/service"));
        assert!(command.stdout && command.strict);
        assert_eq!(command.target, ["other"]);
        assert!(InitCommand::deprecation_warning().contains("ward import <SOURCE>"));
    }

    #[test]
    fn init_has_no_local_parallelism_flag() {
        use clap::Parser;

        let result = crate::cli::Cli::try_parse_from([
            "ward",
            "init",
            "--from",
            "a/b",
            "--parallelism",
            "3",
        ]);
        let cli = result.unwrap();
        assert_eq!(cli.parallelism, 3);
    }

    #[test]
    fn import_uses_the_global_parallelism_flag() {
        use clap::Parser;

        let cli = crate::cli::Cli::parse_from(["ward", "import", "a/b", "--parallelism", "7"]);
        assert_eq!(cli.parallelism, 7);
    }

    #[test]
    fn init_options_are_hidden_from_help() {
        use clap::CommandFactory;

        let mut cli = crate::cli::Cli::command();
        let help = cli
            .find_subcommand_mut("init")
            .unwrap()
            .render_long_help()
            .to_string();
        assert!(help.contains("--non-interactive"), "{help}");
        assert!(!help.contains("--from"), "{help}");
    }

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

        assert!(security.secret_scanning.unwrap());
        assert!(security.secret_scanning_push_protection.unwrap());
        assert!(security.secret_scanning_ai_detection.unwrap());
        assert_eq!(security.policy.disposition, ManagementDisposition::Managed);
        crate::cli::plan::require_canonical_categories(&manifest, "plan").unwrap();
        crate::cli::plan::require_canonical_categories(&manifest, "apply").unwrap();
    }
}
