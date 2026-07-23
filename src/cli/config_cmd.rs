use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use console::style;
use dialoguer::{Confirm, Input};
use toml_edit::DocumentMut;

use crate::config::Manifest;

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
    /// Set a configuration value using dot notation
    Set {
        /// Config key in dot notation (e.g., file_delivery.branch)
        key: String,
        /// Value to set
        value: String,
    },
    /// Add a new system interactively
    AddSystem,
    /// Remove a system by ID
    RemoveSystem {
        /// System ID to remove
        id: String,
        /// Skip confirmation
        #[arg(long, short)]
        yes: bool,
    },
}

const VALID_KEYS: &[(&str, ValueKind)] = &[
    ("org.name", ValueKind::Str),
    ("security.secret_scanning", ValueKind::Bool),
    ("security.push_protection", ValueKind::Bool),
    ("security.dependabot_alerts", ValueKind::Bool),
    ("security.dependabot_security_updates", ValueKind::Bool),
    ("security.secret_scanning_ai_detection", ValueKind::Bool),
    ("security.codeql_advanced_setup", ValueKind::Bool),
    ("branch_protection.enabled", ValueKind::Bool),
    ("branch_protection.required_approvals", ValueKind::Int),
    ("branch_protection.dismiss_stale_reviews", ValueKind::Bool),
    (
        "branch_protection.require_code_owner_reviews",
        ValueKind::Bool,
    ),
    ("branch_protection.require_status_checks", ValueKind::Bool),
    ("branch_protection.strict_status_checks", ValueKind::Bool),
    ("branch_protection.enforce_admins", ValueKind::Bool),
    ("branch_protection.required_linear_history", ValueKind::Bool),
    ("branch_protection.allow_force_pushes", ValueKind::Bool),
    ("branch_protection.allow_deletions", ValueKind::Bool),
    ("file_delivery.branch", ValueKind::Str),
    ("file_delivery.commit_message_prefix", ValueKind::Str),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Bool,
    Int,
    Str,
}

impl ConfigCommand {
    pub fn run(self, config_override: Option<&str>) -> Result<()> {
        match self.action {
            ConfigAction::Show => run_show(config_override),
            ConfigAction::Edit => run_edit(config_override),
            ConfigAction::Path => run_path(config_override),
            ConfigAction::Set { key, value } => {
                let path = resolve_config_path(config_override);
                apply_set(&path, &key, &value)
            }
            ConfigAction::AddSystem => run_add_system(config_override),
            ConfigAction::RemoveSystem { id, yes } => {
                let path = resolve_config_path(config_override);
                if !yes {
                    let confirmed = Confirm::new()
                        .with_prompt(format!("Remove system '{id}'?"))
                        .default(false)
                        .interact()?;
                    if !confirmed {
                        println!("  {} Cancelled.", style("[..]").dim());
                        return Ok(());
                    }
                }
                remove_system_by_id(&path, &id)
            }
        }
    }
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
    println!("  {}", style("Organization").bold());
    println!("    name: {}", style(&manifest.org.name).cyan());

    println!();
    println!("  {}", style("Security").bold());
    print_bool("    secret_scanning", manifest.security.secret_scanning);
    print_bool(
        "    secret_scanning_ai_detection",
        manifest.security.secret_scanning_ai_detection,
    );
    print_bool("    push_protection", manifest.security.push_protection);
    print_bool("    dependabot_alerts", manifest.security.dependabot_alerts);
    print_bool(
        "    dependabot_security_updates",
        manifest.security.dependabot_security_updates,
    );
    print_bool(
        "    codeql_advanced_setup",
        manifest.security.codeql_advanced_setup,
    );

    println!();
    println!("  {}", style("Branch Protection").bold());
    print_bool("    enabled", manifest.branch_protection.enabled);
    println!(
        "    required_approvals: {}",
        manifest.branch_protection.required_approvals
    );
    print_bool(
        "    dismiss_stale_reviews",
        manifest.branch_protection.dismiss_stale_reviews,
    );
    print_bool(
        "    require_code_owner_reviews",
        manifest.branch_protection.require_code_owner_reviews,
    );
    print_bool(
        "    require_status_checks",
        manifest.branch_protection.require_status_checks,
    );
    print_bool(
        "    strict_status_checks",
        manifest.branch_protection.strict_status_checks,
    );
    print_bool(
        "    enforce_admins",
        manifest.branch_protection.enforce_admins,
    );
    print_bool(
        "    required_linear_history",
        manifest.branch_protection.required_linear_history,
    );
    print_bool(
        "    allow_force_pushes",
        manifest.branch_protection.allow_force_pushes,
    );
    print_bool(
        "    allow_deletions",
        manifest.branch_protection.allow_deletions,
    );

    println!();
    println!("  {}", style("Rulesets").bold());
    if let Some(ref bp) = manifest.rulesets.branch_protection {
        println!("    {}", style("branch_protection:").dim());
        print_bool("      enabled", bp.enabled);
        println!("      required_approvals: {}", bp.required_approvals);
        print_bool("      dismiss_stale_reviews", bp.dismiss_stale_reviews);
        print_bool(
            "      require_code_owner_reviews",
            bp.require_code_owner_reviews,
        );
        print_bool("      require_linear_history", bp.require_linear_history);
        print_bool("      block_force_pushes", bp.block_force_pushes);
        if !bp.bypass_teams.is_empty() {
            println!(
                "      bypass_teams: {}",
                bp.bypass_teams
                    .iter()
                    .map(|t| t.slug().to_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !bp.overrides.is_empty() {
            println!("      overrides: {} pattern(s)", bp.overrides.len());
        }
    } else {
        println!("    (not configured)");
    }

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

fn print_bool(label: &str, value: bool) {
    if value {
        println!("{label}: {}", style("true").green());
    } else {
        println!("{label}: {}", style("false").red());
    }
}

fn run_edit(config_override: Option<&str>) -> Result<()> {
    let path = resolve_config_path(config_override);
    if !path.exists() {
        bail!(
            "No configuration file at {}. Run `ward init` first.",
            path.display()
        );
    }

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

fn run_path(config_override: Option<&str>) -> Result<()> {
    let path = resolve_config_path(config_override);
    let abs = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    println!("{}", abs.display());
    if path.exists() {
        println!("  {} File exists.", style("[ok]").green());
    } else {
        println!("  {} File does not exist.", style("[..]").yellow());
    }
    Ok(())
}

fn lookup_key(key: &str) -> Option<ValueKind> {
    VALID_KEYS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

pub fn apply_set(path: &Path, key: &str, value: &str) -> Result<()> {
    let kind = lookup_key(key).ok_or_else(|| {
        anyhow::anyhow!(
            "Unknown config key '{key}'. Valid keys:\n  {}",
            VALID_KEYS
                .iter()
                .map(|(k, _)| *k)
                .collect::<Vec<_>>()
                .join("\n  ")
        )
    })?;

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("Failed to parse {}", path.display()))?;

    let parts: Vec<&str> = key.splitn(2, '.').collect();
    if parts.len() != 2 {
        bail!("Key must use dot notation (e.g., security.push_protection)");
    }
    let (section, field) = (parts[0], parts[1]);

    if doc.get(section).is_none() {
        doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    match kind {
        ValueKind::Bool => {
            let parsed: bool = value
                .parse()
                .with_context(|| format!("Expected bool for '{key}', got '{value}'"))?;
            doc[section][field] = toml_edit::value(parsed);
        }
        ValueKind::Int => {
            let parsed: i64 = value
                .parse()
                .with_context(|| format!("Expected integer for '{key}', got '{value}'"))?;
            doc[section][field] = toml_edit::value(parsed);
        }
        ValueKind::Str => {
            doc[section][field] = toml_edit::value(value);
        }
    }

    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Failed to write {}", path.display()))?;

    println!("  {} Set {key} = {value}", style("[ok]").green());
    Ok(())
}

fn run_add_system(config_override: Option<&str>) -> Result<()> {
    let path = resolve_config_path(config_override);
    if !path.exists() {
        bail!(
            "No configuration file at {}. Run `ward init` first.",
            path.display()
        );
    }

    let id: String = Input::new().with_prompt("  System ID").interact_text()?;

    let name: String = Input::new()
        .with_prompt("  Display name")
        .default(id.clone())
        .interact_text()?;

    let exclude_raw: String = Input::new()
        .with_prompt("  Exclude patterns (comma-separated, optional)")
        .default(String::new())
        .allow_empty(true)
        .interact_text()?;

    let exclude: Vec<String> = exclude_raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();

    let repos_raw: String = Input::new()
        .with_prompt("  Explicit repos (comma-separated, optional)")
        .default(String::new())
        .allow_empty(true)
        .interact_text()?;

    let repos: Vec<String> = repos_raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();

    append_system(&path, &id, &name, &exclude, &repos)?;
    println!(
        "  {} Added system {} ({})",
        style("[ok]").green(),
        style(&name).cyan(),
        id,
    );
    Ok(())
}

pub fn append_system(
    path: &Path,
    id: &str,
    name: &str,
    exclude: &[String],
    repos: &[String],
) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let manifest: Manifest =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    if manifest.systems.iter().any(|s| s.id == id) {
        bail!("System '{id}' already exists");
    }

    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("Failed to parse {}", path.display()))?;

    let systems = doc
        .entry("systems")
        .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));

    let arr = systems
        .as_array_of_tables_mut()
        .ok_or_else(|| anyhow::anyhow!("'systems' is not an array of tables"))?;

    let mut table = toml_edit::Table::new();
    table.insert("id", toml_edit::value(id));
    table.insert("name", toml_edit::value(name));
    if !exclude.is_empty() {
        let mut arr_val = toml_edit::Array::new();
        for e in exclude {
            arr_val.push(e.as_str());
        }
        table.insert("exclude", toml_edit::value(arr_val));
    }
    if !repos.is_empty() {
        let mut arr_val = toml_edit::Array::new();
        for r in repos {
            arr_val.push(r.as_str());
        }
        table.insert("repos", toml_edit::value(arr_val));
    }

    arr.push(table);

    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Failed to write {}", path.display()))?;

    Ok(())
}

pub fn remove_system_by_id(path: &Path, id: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("Failed to parse {}", path.display()))?;

    let systems = doc
        .get_mut("systems")
        .and_then(|s| s.as_array_of_tables_mut())
        .ok_or_else(|| anyhow::anyhow!("No [[systems]] found in configuration"))?;

    let idx = systems
        .iter()
        .position(|t| t.get("id").and_then(|v| v.as_str()) == Some(id))
        .ok_or_else(|| anyhow::anyhow!("System '{id}' not found"))?;

    systems.remove(idx);

    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Failed to write {}", path.display()))?;

    println!(
        "  {} Removed system {}",
        style("[ok]").green(),
        style(id).cyan()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    const SAMPLE_TOML: &str = r#"# Ward configuration
[org]
name = "my-org"

[security]
# Enable secret scanning
secret_scanning = true
push_protection = false
dependabot_alerts = true
dependabot_security_updates = true

[branch_protection]
enabled = true
required_approvals = 1

[file_delivery]
branch = "chore/ward-setup"
commit_message_prefix = "chore: "

[[systems]]
id = "backend"
name = "Backend Services"
exclude = ["operations?"]
"#;

    fn write_temp(content: &str) -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), content).unwrap();
        file
    }

    #[test]
    fn test_config_set_bool_value() {
        let file = write_temp(SAMPLE_TOML);
        apply_set(file.path(), "security.push_protection", "true").unwrap();

        let updated = std::fs::read_to_string(file.path()).unwrap();
        let manifest: Manifest = toml::from_str(&updated).unwrap();
        assert!(manifest.security.push_protection);
    }

    #[test]
    fn test_config_set_string_value() {
        let file = write_temp(SAMPLE_TOML);
        apply_set(file.path(), "org.name", "new-org").unwrap();

        let updated = std::fs::read_to_string(file.path()).unwrap();
        let manifest: Manifest = toml::from_str(&updated).unwrap();
        assert_eq!(manifest.org.name, "new-org");
    }

    #[test]
    fn test_config_set_integer_value() {
        let file = write_temp(SAMPLE_TOML);
        apply_set(file.path(), "branch_protection.required_approvals", "3").unwrap();

        let updated = std::fs::read_to_string(file.path()).unwrap();
        let manifest: Manifest = toml::from_str(&updated).unwrap();
        assert_eq!(manifest.branch_protection.required_approvals, 3);
    }

    #[test]
    fn test_config_set_preserves_comments() {
        let file = write_temp(SAMPLE_TOML);
        apply_set(file.path(), "security.push_protection", "true").unwrap();

        let updated = std::fs::read_to_string(file.path()).unwrap();
        assert!(
            updated.contains("# Ward configuration"),
            "Top-level comment should be preserved"
        );
        assert!(
            updated.contains("# Enable secret scanning"),
            "Inline comment should be preserved"
        );
    }

    #[test]
    fn test_config_set_invalid_key() {
        let file = write_temp(SAMPLE_TOML);
        let result = apply_set(file.path(), "nonexistent.key", "value");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown config key")
        );
    }

    #[test]
    fn test_config_add_system_to_toml() {
        let file = write_temp(SAMPLE_TOML);
        append_system(
            file.path(),
            "frontend",
            "Frontend Apps",
            &["workflows".to_owned()],
            &[],
        )
        .unwrap();

        let updated = std::fs::read_to_string(file.path()).unwrap();
        let manifest: Manifest = toml::from_str(&updated).unwrap();
        assert_eq!(manifest.systems.len(), 2);
        assert_eq!(manifest.systems[1].id, "frontend");
        assert_eq!(manifest.systems[1].name, "Frontend Apps");
        assert_eq!(manifest.systems[1].exclude, vec!["workflows"]);
    }

    #[test]
    fn test_config_add_system_rejects_duplicate() {
        let file = write_temp(SAMPLE_TOML);
        let result = append_system(file.path(), "backend", "Duplicate", &[], &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_config_remove_system_from_toml() {
        let file = write_temp(SAMPLE_TOML);
        remove_system_by_id(file.path(), "backend").unwrap();

        let updated = std::fs::read_to_string(file.path()).unwrap();
        let manifest: Manifest = toml::from_str(&updated).unwrap();
        assert!(manifest.systems.is_empty());
    }

    #[test]
    fn test_config_remove_nonexistent_system() {
        let file = write_temp(SAMPLE_TOML);
        let result = remove_system_by_id(file.path(), "nope");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

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
}
