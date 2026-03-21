use serde::Serialize;

use crate::config::SecurityConfig;
use crate::github::security::SecurityState;

/// A planned change for a single security feature.
#[derive(Debug, Clone, Serialize)]
pub struct SecurityChange {
    pub feature: String,
    pub current: bool,
    pub desired: bool,
}

/// A plan for a single repository.
#[derive(Debug, Clone, Serialize)]
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
                feature: feature.to_owned(),
                current: current_val,
                desired: desired_val,
            });
        }
    }

    RepoPlan {
        repo: repo.to_owned(),
        changes,
    }
}
