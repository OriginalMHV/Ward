use serde::Serialize;

use crate::config::SecurityConfig;
use crate::github::security::SecurityState;

/// A planned change for a single security feature.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SecurityChange {
    pub feature: String,
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
            "dependabot_alerts",
            current.dependabot_alerts,
            desired.dependabot_alerts,
        ),
        (
            "dependabot_security_updates",
            current.dependabot_security_updates,
            desired.dependabot_security_updates,
        ),
        (
            "secret_scanning",
            current.secret_scanning,
            desired.secret_scanning,
        ),
        (
            "secret_scanning_ai_detection",
            current.secret_scanning_ai_detection,
            desired.secret_scanning_ai_detection,
        ),
        (
            "push_protection",
            current.push_protection,
            desired.push_protection,
        ),
    ];

    for (feature, current_val, desired_val) in checks {
        if current_val != desired_val {
            changes.push(SecurityChange {
                feature: feature.to_string(),
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
            checks: vec![],
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
        let features: Vec<&str> = plan.changes.iter().map(|c| c.feature.as_str()).collect();
        assert!(features.contains(&"dependabot_security_updates"));
        assert!(features.contains(&"secret_scanning_ai_detection"));
        assert!(features.contains(&"push_protection"));
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
}
