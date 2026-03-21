use serde::Serialize;

/// Detected project type for a repository.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum ProjectType {
    Gradle,
    Npm,
    Cargo,
    Unknown,
}

impl ProjectType {
    /// Detect from the presence of known build files.
    pub fn from_files(files: &[String]) -> Self {
        const DETECTORS: &[(&[&str], ProjectType)] = &[
            (&["build.gradle.kts", "build.gradle"], ProjectType::Gradle),
            (&["package.json"], ProjectType::Npm),
            (&["Cargo.toml"], ProjectType::Cargo),
        ];

        DETECTORS
            .iter()
            .find(|(markers, _)| markers.iter().any(|m| files.iter().any(|f| f == m)))
            .map(|(_, pt)| pt.clone())
            .unwrap_or(Self::Unknown)
    }
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gradle => write!(f, "Gradle"),
            Self::Npm => write!(f, "npm"),
            Self::Cargo => write!(f, "Cargo"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detect_gradle_kts() {
        assert_eq!(ProjectType::from_files(&files(&["build.gradle.kts", "settings.gradle.kts"])), ProjectType::Gradle);
    }

    #[test]
    fn detect_gradle_groovy() {
        assert_eq!(ProjectType::from_files(&files(&["build.gradle"])), ProjectType::Gradle);
    }

    #[test]
    fn detect_npm() {
        assert_eq!(ProjectType::from_files(&files(&["package.json", "package-lock.json"])), ProjectType::Npm);
    }

    #[test]
    fn detect_cargo() {
        assert_eq!(ProjectType::from_files(&files(&["Cargo.toml", "Cargo.lock"])), ProjectType::Cargo);
    }

    #[test]
    fn detect_unknown_for_empty() {
        assert_eq!(ProjectType::from_files(&[]), ProjectType::Unknown);
    }

    #[test]
    fn detect_unknown_for_unrecognized() {
        assert_eq!(ProjectType::from_files(&files(&["Makefile", "go.mod"])), ProjectType::Unknown);
    }

    #[test]
    fn gradle_takes_priority_over_npm() {
        assert_eq!(ProjectType::from_files(&files(&["build.gradle.kts", "package.json"])), ProjectType::Gradle);
    }

    #[test]
    fn display_formatting() {
        assert_eq!(format!("{}", ProjectType::Gradle), "Gradle");
        assert_eq!(format!("{}", ProjectType::Npm), "npm");
        assert_eq!(format!("{}", ProjectType::Cargo), "Cargo");
        assert_eq!(format!("{}", ProjectType::Unknown), "Unknown");
    }
}
