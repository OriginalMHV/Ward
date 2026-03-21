use anyhow::Result;
use rust_embed::Embed;
use tera::Tera;

#[derive(Embed)]
#[folder = "templates/"]
pub struct TemplateAssets;

/// Load embedded templates into a Tera instance.
pub fn load_templates() -> Result<Tera> {
    let mut tera = Tera::default();

    for file in TemplateAssets::iter() {
        let path = file.as_ref();
        if let Some(content) = TemplateAssets::get(path) {
            let text = std::str::from_utf8(content.data.as_ref())?;
            tera.add_raw_template(path, text)?;
        }
    }

    Ok(tera)
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
        let result = tera.render("dependency-submission/gradle.yml.tera", &ctx).unwrap();
        assert!(result.contains("JAVA_VERSION: 21"));
        assert!(result.contains("branches:"));
    }
}
