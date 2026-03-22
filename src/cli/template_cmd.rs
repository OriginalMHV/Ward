use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use console::style;

use crate::config::templates::TemplateAssets;

#[derive(clap::Args)]
pub struct TemplateCommand {
    #[command(subcommand)]
    pub action: TemplateAction,
}

#[derive(clap::Subcommand)]
pub enum TemplateAction {
    /// List all available templates
    List,
    /// Show template content
    Show {
        /// Template name (e.g., codeql/gradle.yml.tera)
        name: String,
    },
    /// Export embedded templates to custom directory for editing
    Export {
        /// Template name to export (exports all if omitted)
        name: Option<String>,
    },
    /// Create a new custom template
    Create {
        /// Template path relative to templates dir (e.g., custom/my-workflow.yml.tera)
        path: String,
    },
    /// Show custom templates directory
    Dir,
}

impl TemplateCommand {
    pub fn run(self, _config_override: Option<&str>) -> Result<()> {
        match self.action {
            TemplateAction::List => run_list(),
            TemplateAction::Show { name } => run_show(&name),
            TemplateAction::Export { name } => {
                let dir = templates_dir()?;
                match name {
                    Some(n) => export_single(&n, &dir),
                    None => export_all(&dir),
                }
            }
            TemplateAction::Create { path } => run_create(&path),
            TemplateAction::Dir => run_dir(),
        }
    }
}

fn templates_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home).join(".ward").join("templates"))
}

fn embedded_templates() -> Vec<String> {
    let mut names: Vec<String> = TemplateAssets::iter().map(|f| f.to_string()).collect();
    names.sort();
    names
}

fn custom_templates(dir: &Path) -> Vec<String> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut results = Vec::new();
    collect_custom(dir, dir, &mut results);
    results.sort();
    results
}

fn collect_custom(base: &Path, current: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_custom(base, &path, out);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().to_string());
            }
        }
    }
}

fn category_of(name: &str) -> &str {
    name.split('/').next().unwrap_or("other")
}

fn run_list() -> Result<()> {
    let embedded = embedded_templates();
    let dir = templates_dir()?;
    let custom = custom_templates(&dir);

    let custom_set: std::collections::HashSet<&str> = custom.iter().map(|s| s.as_str()).collect();
    let embedded_set: std::collections::HashSet<&str> =
        embedded.iter().map(|s| s.as_str()).collect();

    let mut all: Vec<(&str, &str)> = Vec::new();

    for name in &embedded {
        if custom_set.contains(name.as_str()) {
            all.push((name, "override"));
        } else {
            all.push((name, "built-in"));
        }
    }

    for name in &custom {
        if !embedded_set.contains(name.as_str()) {
            all.push((name, "custom"));
        }
    }

    all.sort_by(|a, b| a.0.cmp(b.0));

    let mut current_category = "";
    for (name, source) in &all {
        let cat = category_of(name);
        if cat != current_category {
            println!();
            println!("  {}", style(cat).bold());
            current_category = cat;
        }
        let tag = match *source {
            "built-in" => style("[built-in]").dim(),
            "custom" => style("[custom]").green(),
            "override" => style("[override]").yellow(),
            _ => style("[unknown]").dim(),
        };
        println!("    {name}  {tag}");
    }
    println!();
    Ok(())
}

fn run_show(name: &str) -> Result<()> {
    let dir = templates_dir()?;
    let custom_path = dir.join(name);

    if custom_path.is_file() {
        let content = std::fs::read_to_string(&custom_path)
            .with_context(|| format!("Failed to read {}", custom_path.display()))?;
        print!("{content}");
        return Ok(());
    }

    match TemplateAssets::get(name) {
        Some(file) => {
            let content = std::str::from_utf8(file.data.as_ref())
                .context("Template content is not valid UTF-8")?;
            print!("{content}");
            Ok(())
        }
        None => bail!("Template '{name}' not found"),
    }
}

pub fn export_single(name: &str, dest_dir: &Path) -> Result<()> {
    let file = TemplateAssets::get(name)
        .ok_or_else(|| anyhow::anyhow!("Embedded template '{name}' not found"))?;

    let dest = dest_dir.join(name);
    if dest.exists() {
        println!(
            "  {} Skipping {} (custom file already exists)",
            style("[..]").yellow(),
            name,
        );
        return Ok(());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let content =
        std::str::from_utf8(file.data.as_ref()).context("Template content is not valid UTF-8")?;
    std::fs::write(&dest, content)
        .with_context(|| format!("Failed to write {}", dest.display()))?;

    println!("  {} {}", style("[ok]").green(), dest.display());
    Ok(())
}

pub fn export_all(dest_dir: &Path) -> Result<()> {
    let names = embedded_templates();
    let mut exported = 0usize;
    let mut skipped = 0usize;

    for name in &names {
        let dest = dest_dir.join(name);
        if dest.exists() {
            println!(
                "  {} Skipping {} (already exists)",
                style("[..]").yellow(),
                name,
            );
            skipped += 1;
            continue;
        }

        let file = TemplateAssets::get(name).unwrap();
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = std::str::from_utf8(file.data.as_ref())?;
        std::fs::write(&dest, content)?;
        println!("  {} {}", style("[ok]").green(), dest.display());
        exported += 1;
    }

    println!();
    println!("  Exported {exported} template(s), skipped {skipped}.",);
    Ok(())
}

fn run_create(template_path: &str) -> Result<()> {
    let dir = templates_dir()?;
    let dest = dir.join(template_path);

    if dest.exists() {
        bail!("Template already exists at {}", dest.display());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    std::fs::write(&dest, "").with_context(|| format!("Failed to create {}", dest.display()))?;

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_owned());

    let _ = std::process::Command::new(&editor).arg(&dest).status();

    println!(
        "  {} Created template at {}",
        style("[ok]").green(),
        dest.display(),
    );
    Ok(())
}

fn run_dir() -> Result<()> {
    let dir = templates_dir()?;

    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create {}", dir.display()))?;
        println!("{}", dir.display());
        println!("  {} Directory created (empty).", style("[ok]").green());
        return Ok(());
    }

    let custom = custom_templates(&dir);
    println!("{}", dir.display());
    if custom.is_empty() {
        println!("  {} No custom templates.", style("[..]").dim());
    } else {
        println!(
            "  {} {} custom template(s).",
            style("[ok]").green(),
            custom.len(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_list_embedded_templates() {
        let templates = embedded_templates();
        assert!(
            !templates.is_empty(),
            "Should have at least one embedded template"
        );
        assert!(
            templates.iter().any(|t| t.contains("codeql")),
            "Should include codeql templates"
        );
        assert!(
            templates.iter().any(|t| t.contains("dependabot")),
            "Should include dependabot templates"
        );
    }

    #[test]
    fn test_export_single_template() {
        let dir = TempDir::new().unwrap();
        let templates = embedded_templates();
        let name = &templates[0];

        export_single(name, dir.path()).unwrap();

        let exported = dir.path().join(name);
        assert!(exported.exists(), "Exported file should exist");

        let content = std::fs::read_to_string(&exported).unwrap();
        let embedded = TemplateAssets::get(name).unwrap();
        let expected = std::str::from_utf8(embedded.data.as_ref()).unwrap();
        assert_eq!(content, expected, "Content should match embedded template");
    }

    #[test]
    fn test_export_all_templates() {
        let dir = TempDir::new().unwrap();
        export_all(dir.path()).unwrap();

        let expected_count = embedded_templates().len();
        let exported = custom_templates(dir.path());
        assert_eq!(
            exported.len(),
            expected_count,
            "All embedded templates should be exported"
        );
    }

    #[test]
    fn test_export_skip_existing() {
        let dir = TempDir::new().unwrap();
        let templates = embedded_templates();
        let name = &templates[0];

        export_single(name, dir.path()).unwrap();

        let path = dir.path().join(name);
        std::fs::write(&path, "custom content").unwrap();

        export_single(name, dir.path()).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content, "custom content",
            "Should not overwrite existing file"
        );
    }

    #[test]
    fn test_create_template() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("custom").join("test.yml.tera");

        assert!(!dest.exists());

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&dest, "").unwrap();

        assert!(dest.exists(), "Template file should be created");
    }

    #[test]
    fn test_embedded_template_content_is_valid_utf8() {
        for name in embedded_templates() {
            let file = TemplateAssets::get(&name).unwrap();
            assert!(
                std::str::from_utf8(file.data.as_ref()).is_ok(),
                "Template '{name}' should be valid UTF-8"
            );
        }
    }

    #[test]
    fn test_category_of_extracts_first_segment() {
        assert_eq!(category_of("codeql/gradle.yml.tera"), "codeql");
        assert_eq!(category_of("dependabot/npm.yml.tera"), "dependabot");
        assert_eq!(category_of("standalone.yml"), "standalone.yml");
    }
}
