use std::path::Path;

use anyhow::{Context, Result};
use rust_embed::Embed;
use tera::Tera;

#[derive(Embed)]
#[folder = "templates/"]
pub struct TemplateAssets;

/// Load embedded templates into a Tera instance.
pub fn load_templates() -> Result<Tera> {
    load_templates_with_custom_dir(None)
}

/// Load embedded templates, then overlay templates from a custom directory.
/// Custom templates with the same name override embedded ones.
pub fn load_templates_with_custom_dir(custom_dir: Option<&Path>) -> Result<Tera> {
    let mut tera = Tera::default();

    for file in TemplateAssets::iter() {
        let path = file.as_ref();
        if let Some(content) = TemplateAssets::get(path) {
            let text = std::str::from_utf8(content.data.as_ref())?;
            tera.add_raw_template(path, text)?;
        }
    }

    if let Some(dir) = custom_dir {
        if dir.is_dir() {
            load_custom_dir(&mut tera, dir)?;
        }
    } else if let Some(default_dir) = dirs_default_templates()
        && default_dir.is_dir()
    {
        load_custom_dir(&mut tera, &default_dir)?;
    }

    Ok(tera)
}

fn dirs_default_templates() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".ward").join("templates"))
}

fn load_custom_dir(tera: &mut Tera, dir: &Path) -> Result<()> {
    walk_dir(tera, dir, dir)
}

fn walk_dir(tera: &mut Tera, base: &Path, current: &Path) -> Result<()> {
    for entry in std::fs::read_dir(current)
        .with_context(|| format!("Failed to read custom template dir: {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            walk_dir(tera, base, &path)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(base).with_context(|| {
                format!("Failed to compute relative path for {}", path.display())
            })?;
            let name = rel.to_string_lossy();
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read custom template: {}", path.display()))?;
            tera.add_raw_template(&name, &text)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_embedded_templates() {
        let tera = load_templates().unwrap();
        let names: Vec<&str> = tera.get_template_names().collect();
        assert!(names.iter().any(|n| n.contains("dependabot")));
        assert!(names.iter().any(|n| n.contains("codeql")));
        assert!(names.iter().any(|n| n.contains("dependency-submission")));
        assert!(names.iter().any(|n| n.contains("copilot-review")));
    }

    #[test]
    fn loads_with_nonexistent_custom_dir() {
        let tera =
            load_templates_with_custom_dir(Some(Path::new("/nonexistent/path/templates"))).unwrap();
        let names: Vec<&str> = tera.get_template_names().collect();
        assert!(names.iter().any(|n| n.contains("dependabot")));
    }

    #[test]
    fn custom_dir_overrides_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("dependabot");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join("gradle.yml.tera"),
            "custom: {{ custom_var | default(value='hello') }}",
        )
        .unwrap();

        let tera = load_templates_with_custom_dir(Some(dir.path())).unwrap();
        let ctx = tera::Context::new();
        let result = tera.render("dependabot/gradle.yml.tera", &ctx).unwrap();
        assert_eq!(result, "custom: hello");
    }

    #[test]
    fn custom_dir_adds_new_templates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my-custom.tera"), "hello world").unwrap();

        let tera = load_templates_with_custom_dir(Some(dir.path())).unwrap();
        let names: Vec<&str> = tera.get_template_names().collect();
        assert!(names.contains(&"my-custom.tera"));

        let ctx = tera::Context::new();
        let result = tera.render("my-custom.tera", &ctx).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn custom_dir_walks_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.tera"), "deep template").unwrap();

        let tera = load_templates_with_custom_dir(Some(dir.path())).unwrap();
        let ctx = tera::Context::new();
        let result = tera.render("a/b/deep.tera", &ctx).unwrap();
        assert_eq!(result, "deep template");
    }

    #[test]
    fn renders_dependabot_gradle_template() {
        let tera = load_templates().unwrap();
        let mut ctx = tera::Context::new();
        ctx.insert("registry_url", "https://example.com/maven");
        ctx.insert("jfrog_oidc_provider", "my-provider");
        let result = tera.render("dependabot/gradle.yml.tera", &ctx).unwrap();
        assert!(result.contains("https://example.com/maven"));
        assert!(result.contains("my-provider"));
        assert!(result.contains("package-ecosystem: gradle"));
    }

    #[test]
    fn renders_dependabot_gradle_with_defaults() {
        let tera = load_templates().unwrap();
        let ctx = tera::Context::new();
        let result = tera.render("dependabot/gradle.yml.tera", &ctx).unwrap();
        assert!(result.contains("https://repo.maven.apache.org/maven2"));
        assert!(!result.contains("jfrog-oidc-provider-name"));
    }

    #[test]
    fn renders_codeql_gradle_template() {
        let tera = load_templates().unwrap();
        let mut ctx = tera::Context::new();
        ctx.insert("java_version", "17");
        let result = tera.render("codeql/gradle.yml.tera", &ctx).unwrap();
        assert!(result.contains("JAVA_VERSION: 17"));
        assert!(result.contains("java-kotlin"));
    }

    #[test]
    fn renders_codeql_npm_template() {
        let tera = load_templates().unwrap();
        let mut ctx = tera::Context::new();
        ctx.insert("node_version", "20");
        let result = tera.render("codeql/npm.yml.tera", &ctx).unwrap();
        assert!(result.contains("NODE_VERSION: 20"));
        assert!(result.contains("javascript-typescript"));
    }

    #[test]
    fn renders_dependency_submission_template() {
        let tera = load_templates().unwrap();
        let mut ctx = tera::Context::new();
        ctx.insert("java_version", "21");
        ctx.insert("default_branch", "main");
        let result = tera
            .render("dependency-submission/gradle.yml.tera", &ctx)
            .unwrap();
        assert!(result.contains("JAVA_VERSION: 21"));
        assert!(result.contains("branches:"));
    }
}
