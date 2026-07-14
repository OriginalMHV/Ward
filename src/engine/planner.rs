use std::fmt;

use serde::Serialize;

use crate::config::SecurityConfig;
use crate::github::security::SecurityState;

/// The set of security features Ward can manage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFeature {
    DependabotAlerts,
    DependabotSecurityUpdates,
    SecretScanning,
    SecretScanningAiDetection,
    PushProtection,
}

impl fmt::Display for SecurityFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DependabotAlerts => write!(f, "dependabot_alerts"),
            Self::DependabotSecurityUpdates => write!(f, "dependabot_security_updates"),
            Self::SecretScanning => write!(f, "secret_scanning"),
            Self::SecretScanningAiDetection => write!(f, "secret_scanning_ai_detection"),
            Self::PushProtection => write!(f, "push_protection"),
        }
    }
}

impl SecurityFeature {
    /// All known features, in canonical order.
    pub const ALL: [SecurityFeature; 5] = [
        Self::DependabotAlerts,
        Self::DependabotSecurityUpdates,
        Self::SecretScanning,
        Self::SecretScanningAiDetection,
        Self::PushProtection,
    ];

    /// Whether this feature is part of the secret-scanning PATCH group.
    pub fn is_secret_scanning_group(&self) -> bool {
        matches!(
            self,
            Self::SecretScanning | Self::SecretScanningAiDetection | Self::PushProtection
        )
    }
}

/// A planned change for a single security feature.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SecurityChange {
    pub feature: SecurityFeature,
    pub current: bool,
    pub desired: bool,
}

/// A plan for a single repository.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoPlan {
    pub repo: String,
    pub changes: Vec<SecurityChange>,
}

impl RepoPlan {
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }
}

/// Diff the current security state against the desired config.
pub fn plan_security(repo: &str, current: &SecurityState, desired: &SecurityConfig) -> RepoPlan {
    let mut changes = Vec::new();

    let checks = [
        (
            SecurityFeature::DependabotAlerts,
            current.dependabot_alerts,
            desired.dependabot_alerts,
        ),
        (
            SecurityFeature::DependabotSecurityUpdates,
            current.dependabot_security_updates,
            desired.dependabot_security_updates,
        ),
        (
            SecurityFeature::SecretScanning,
            current.secret_scanning,
            desired.secret_scanning,
        ),
        (
            SecurityFeature::SecretScanningAiDetection,
            current.secret_scanning_ai_detection,
            desired.secret_scanning_ai_detection,
        ),
        (
            SecurityFeature::PushProtection,
            current.push_protection,
            desired.push_protection,
        ),
    ];

    for (feature, current_val, desired_val) in checks {
        if current_val != desired_val {
            changes.push(SecurityChange {
                feature,
                current: current_val,
                desired: desired_val,
            });
        }
    }

    RepoPlan {
        repo: repo.to_string(),
        changes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(da: bool, dsu: bool, ss: bool, ai: bool, pp: bool) -> SecurityState {
        SecurityState {
            dependabot_alerts: da,
            dependabot_security_updates: dsu,
            secret_scanning: ss,
            secret_scanning_ai_detection: ai,
            push_protection: pp,
        }
    }

    fn config(da: bool, dsu: bool, ss: bool, ai: bool, pp: bool) -> SecurityConfig {
        SecurityConfig {
            dependabot_alerts: da,
            dependabot_security_updates: dsu,
            secret_scanning: ss,
            secret_scanning_ai_detection: ai,
            push_protection: pp,
            codeql_advanced_setup: false,
        }
    }

    #[test]
    fn no_changes_when_state_matches_config() {
        let plan = plan_security(
            "repo",
            &state(true, true, true, true, true),
            &config(true, true, true, true, true),
        );
        assert!(!plan.has_changes());
        assert!(plan.changes.is_empty());
    }

    #[test]
    fn all_changes_when_nothing_enabled() {
        let plan = plan_security(
            "repo",
            &state(false, false, false, false, false),
            &config(true, true, true, true, true),
        );
        assert!(plan.has_changes());
        assert_eq!(plan.changes.len(), 5);
        assert!(plan.changes.iter().all(|c| !c.current && c.desired));
    }

    #[test]
    fn partial_changes() {
        let plan = plan_security(
            "repo",
            &state(true, false, true, false, false),
            &config(true, true, true, true, true),
        );
        assert_eq!(plan.changes.len(), 3);
        let features: Vec<SecurityFeature> = plan.changes.iter().map(|c| c.feature).collect();
        assert!(features.contains(&SecurityFeature::DependabotSecurityUpdates));
        assert!(features.contains(&SecurityFeature::SecretScanningAiDetection));
        assert!(features.contains(&SecurityFeature::PushProtection));
    }

    #[test]
    fn plan_to_disable_features() {
        let plan = plan_security(
            "repo",
            &state(true, true, true, true, true),
            &config(false, false, false, false, false),
        );
        assert_eq!(plan.changes.len(), 5);
        assert!(plan.changes.iter().all(|c| c.current && !c.desired));
    }

    #[test]
    fn repo_name_preserved() {
        let plan = plan_security(
            "my-cool-repo",
            &state(false, false, false, false, false),
            &config(true, true, true, true, true),
        );
        assert_eq!(plan.repo, "my-cool-repo");
    }

    #[test]
    fn security_feature_display_matches_snake_case() {
        assert_eq!(
            SecurityFeature::DependabotAlerts.to_string(),
            "dependabot_alerts"
        );
        assert_eq!(
            SecurityFeature::DependabotSecurityUpdates.to_string(),
            "dependabot_security_updates"
        );
        assert_eq!(
            SecurityFeature::SecretScanning.to_string(),
            "secret_scanning"
        );
        assert_eq!(
            SecurityFeature::SecretScanningAiDetection.to_string(),
            "secret_scanning_ai_detection"
        );
        assert_eq!(
            SecurityFeature::PushProtection.to_string(),
            "push_protection"
        );
    }

    #[test]
    fn security_feature_all_covers_every_variant() {
        assert_eq!(SecurityFeature::ALL.len(), 5);
    }

    #[test]
    fn security_feature_secret_scanning_group() {
        assert!(!SecurityFeature::DependabotAlerts.is_secret_scanning_group());
        assert!(!SecurityFeature::DependabotSecurityUpdates.is_secret_scanning_group());
        assert!(SecurityFeature::SecretScanning.is_secret_scanning_group());
        assert!(SecurityFeature::SecretScanningAiDetection.is_secret_scanning_group());
        assert!(SecurityFeature::PushProtection.is_secret_scanning_group());
    }
}
