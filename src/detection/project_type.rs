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
        if files
            .iter()
            .any(|f| f == "build.gradle.kts" || f == "build.gradle")
        {
            Self::Gradle
        } else if files.iter().any(|f| f == "package.json") {
            Self::Npm
        } else if files.iter().any(|f| f == "Cargo.toml") {
            Self::Cargo
        } else {
            Self::Unknown
        }
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
