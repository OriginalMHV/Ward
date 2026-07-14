mod common;

use ward::cli::drift::{compare_protection, compare_security};
use ward::config::manifest::{BranchProtectionConfig, SecurityConfig};
use ward::github::branch_protection::BranchProtectionState;
use ward::github::security::SecurityState;

#[tokio::test]
async fn test_drift_check_detects_security_drift() {
    let desired = SecurityConfig {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: true,
        dependabot_alerts: true,
        dependabot_security_updates: true,
        codeql_advanced_setup: false,
    };
    let actual = SecurityState {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: false, // drifted
        dependabot_alerts: true,
        dependabot_security_updates: true,
    };

    let drifts = compare_security(&desired, &actual);

    assert_eq!(drifts.len(), 1);
    assert_eq!(drifts[0].field, "push_protection");
    assert_eq!(drifts[0].expected, "true");
    assert_eq!(drifts[0].actual, "false");
}

#[tokio::test]
async fn test_drift_check_no_drift() {
    let desired_sec = SecurityConfig {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: true,
        dependabot_alerts: true,
        dependabot_security_updates: true,
        codeql_advanced_setup: false,
    };
    let actual_sec = SecurityState {
        secret_scanning: true,
        secret_scanning_ai_detection: true,
        push_protection: true,
        dependabot_alerts: true,
        dependabot_security_updates: true,
    };

    let desired_prot = BranchProtectionConfig {
        enabled: true,
        required_approvals: 1,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        require_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    let actual_prot = BranchProtectionState {
        required_pull_request_reviews: true,
        required_approving_review_count: 1,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        required_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };

    assert!(compare_security(&desired_sec, &actual_sec).is_empty());
    assert!(compare_protection(&desired_prot, &actual_prot).is_empty());
}

#[tokio::test]
async fn test_drift_check_detects_protection_drift() {
    let desired = BranchProtectionConfig {
        enabled: true,
        required_approvals: 2,
        dismiss_stale_reviews: true,
        require_code_owner_reviews: false,
        require_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    let actual = BranchProtectionState {
        required_pull_request_reviews: true,
        required_approving_review_count: 1, // wrong
        dismiss_stale_reviews: false,       // wrong
        require_code_owner_reviews: false,
        required_status_checks: false,
        strict_status_checks: false,
        enforce_admins: false,
        required_linear_history: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };

    let drifts = compare_protection(&desired, &actual);

    assert_eq!(drifts.len(), 2);
    let fields: Vec<&str> = drifts.iter().map(|d| d.field.as_str()).collect();
    assert!(fields.contains(&"dismiss_stale_reviews"));
    assert!(fields.contains(&"required_approvals"));
}
