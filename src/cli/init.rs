use anyhow::Result;
use console::style;

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

pub fn run() -> Result<()> {
    let path = "ward.toml";

    if std::path::Path::new(path).exists() {
        println!("  {} ward.toml already exists.", style("⚠️").yellow());
        return Ok(());
    }

    std::fs::write(path, EXAMPLE_MANIFEST)?;

    println!(
        "  {} Created ward.toml - edit it to configure your org and systems.",
        style("✅").green()
    );

    Ok(())
}
