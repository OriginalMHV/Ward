/// Extract Java toolchain version from build.gradle.kts content.
pub fn extract_java_version(content: &str) -> Option<u8> {
    // Pattern: jvmToolchain(21) or jvmToolchain { languageVersion.set(JavaLanguageVersion.of(21)) }
    for line in content.lines() {
        let trimmed = line.trim();

        // jvmToolchain(21)
        if trimmed.contains("jvmToolchain")
            && let Some(num) = extract_number_from_parens(trimmed)
        {
            return Some(num);
        }

        // JavaLanguageVersion.of(21)
        if let Some(pos) = trimmed.find("JavaLanguageVersion.of(") {
            let after = &trimmed[pos + "JavaLanguageVersion.of(".len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(num) = num_str.parse() {
                return Some(num);
            }
        }

        // sourceCompatibility = JavaVersion.VERSION_21
        if trimmed.contains("sourceCompatibility")
            && trimmed.contains("VERSION_")
            && let Some(v) = trimmed.split("VERSION_").nth(1)
        {
            let cleaned: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(num) = cleaned.parse() {
                return Some(num);
            }
        }
    }

    None
}

/// Extract Node.js version from package.json engines field.
pub fn extract_node_version(content: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(content)
        && let Some(engines) = json.get("engines")
        && let Some(node) = engines.get("node")
    {
        return node.as_str().map(|s| s.to_owned());
    }
    None
}

fn extract_number_from_parens(s: &str) -> Option<u8> {
    if let Some(start) = s.find('(') {
        let rest = &s[start + 1..];
        if let Some(end) = rest.find(')') {
            let num_str: String = rest[..end]
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            return num_str.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jvm_toolchain_simple() {
        assert_eq!(
            extract_java_version("kotlin { jvmToolchain(21) }"),
            Some(21)
        );
    }

    #[test]
    fn test_jvm_toolchain_17() {
        assert_eq!(extract_java_version("    jvmToolchain(17)"), Some(17));
    }

    #[test]
    fn test_java_language_version() {
        let content = r#"
            java {
                toolchain {
                    languageVersion.set(JavaLanguageVersion.of(21))
                }
            }
        "#;
        assert_eq!(extract_java_version(content), Some(21));
    }

    #[test]
    fn test_source_compatibility() {
        assert_eq!(
            extract_java_version("sourceCompatibility = JavaVersion.VERSION_17"),
            Some(17)
        );
    }

    #[test]
    fn test_no_version() {
        assert_eq!(extract_java_version("plugins { id(\"java\") }"), None);
    }

    #[test]
    fn test_node_version() {
        let content = r#"{"engines": {"node": ">=20"}}"#;
        assert_eq!(extract_node_version(content), Some(">=20".to_owned()));
    }

    #[test]
    fn test_no_node_version() {
        let content = r#"{"name": "test"}"#;
        assert_eq!(extract_node_version(content), None);
    }
}
