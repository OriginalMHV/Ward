use super::*;

#[test]
fn parse_minimal_manifest() {
    let toml = r#"
        [org]
        name = "test-org"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.org.name, "test-org");
    // #[serde(default)] on the struct field uses derive(Default), not serde field defaults
    assert!(!m.security.secret_scanning);
    assert!(m.systems.is_empty());
}

#[test]
fn parse_full_manifest() {
    let toml = r#"
        [org]
        name = "my-org"
        [security]
        secret_scanning = false
        push_protection = true
        dependabot_alerts = true
        dependabot_security_updates = false
        [file_delivery]
        branch = "feat/setup"
        reviewers = ["alice"]
        [[systems]]
        id = "backend"
        name = "Backend"
        exclude = ["ops", "infra"]
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.org.name, "my-org");
    assert!(!m.security.secret_scanning);
    assert!(m.security.push_protection);
    assert!(!m.security.dependabot_security_updates);
    assert_eq!(m.file_delivery.branch, "feat/setup");
    assert_eq!(m.file_delivery.reviewers, vec!["alice"]);
    assert_eq!(m.systems.len(), 1);
    assert_eq!(m.systems[0].id, "backend");
    assert_eq!(m.systems[0].exclude, vec!["ops", "infra"]);
}

#[test]
fn system_lookup() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
        [[systems]]
        id = "fe"
        name = "Frontend"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.system("be").unwrap().name, "Backend");
    assert_eq!(m.system("fe").unwrap().name, "Frontend");
    assert!(m.system("missing").is_none());
}

#[test]
fn explicit_repo_matches_system_when_prefix_matching_is_disabled() {
    let toml = r#"
        [org]
        name = "org"

        [[systems]]
        id = "reference"
        name = "Reference"
        match_prefix = false
        repos = ["standalone-service"]
    "#;
    let manifest: Manifest = toml::from_str(toml).unwrap();

    assert_eq!(
        manifest.system_for_repo("standalone-service"),
        Some("reference")
    );
    assert_eq!(manifest.system_for_repo("reference-other"), None);
}

#[test]
fn security_for_system_falls_back_to_global() {
    let toml = r#"
        [org]
        name = "org"
        [security]
        secret_scanning = false
        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert!(!m.security_for_system("be").secret_scanning);
}

#[test]
fn security_for_system_uses_override() {
    let toml = r#"
        [org]
        name = "org"
        [security]
        secret_scanning = true
        [[systems]]
        id = "be"
        name = "Backend"
        [systems.security]
        secret_scanning = false
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert!(!m.security_for_system("be").secret_scanning);
}

#[test]
fn exclude_patterns_for_unknown_system_returns_empty() {
    let m = Manifest::default();
    assert!(m.exclude_patterns_for_system("nope").is_empty());
}

#[test]
fn exclude_patterns_for_known_system() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
        exclude = ["ops", "infra"]
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.exclude_patterns_for_system("be"), vec!["ops", "infra"]);
}

#[test]
fn system_with_explicit_repos() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
        repos = ["standalone-service", "legacy-api"]
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(
        m.explicit_repos_for_system("be"),
        vec!["standalone-service", "legacy-api"]
    );
}

#[test]
fn system_without_explicit_repos_returns_empty() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert!(m.explicit_repos_for_system("be").is_empty());
}

#[test]
fn branch_protection_serde_defaults() {
    let bp: BranchProtectionConfig = toml::from_str("").unwrap();
    assert!(!bp.enabled);
    assert_eq!(bp.required_approvals, 1);
    assert!(!bp.dismiss_stale_reviews);
    assert!(!bp.require_code_owner_reviews);
    assert!(!bp.require_status_checks);
    assert!(!bp.strict_status_checks);
    assert!(!bp.enforce_admins);
    assert!(!bp.required_linear_history);
    assert!(!bp.allow_force_pushes);
    assert!(!bp.allow_deletions);
}

#[test]
fn branch_protection_full_parse() {
    let toml_str = r#"
        enabled = true
        required_approvals = 2
        dismiss_stale_reviews = true
        require_code_owner_reviews = true
        enforce_admins = true
    "#;
    let bp: BranchProtectionConfig = toml::from_str(toml_str).unwrap();
    assert!(bp.enabled);
    assert_eq!(bp.required_approvals, 2);
    assert!(bp.dismiss_stale_reviews);
    assert!(bp.require_code_owner_reviews);
    assert!(bp.enforce_admins);
    assert!(!bp.allow_force_pushes);
}

#[test]
fn default_file_delivery_config_values() {
    // derive(Default) gives empty strings/vecs, not the serde defaults
    let fd = FileDeliveryConfig::default();
    assert_eq!(fd.branch, "");
    assert_eq!(fd.commit_message_prefix, "");
    assert!(fd.reviewers.is_empty());
}

#[test]
fn serde_file_delivery_config_defaults() {
    // When deserialized with missing fields, serde uses the custom defaults
    let fd: FileDeliveryConfig = toml::from_str("").unwrap();
    assert_eq!(fd.branch, "chore/ward-setup");
    assert_eq!(fd.commit_message_prefix, "chore: ");
    assert!(fd.reviewers.is_empty());
}

#[test]
fn default_security_config_all_false() {
    // derive(Default) sets all bools to false
    let sc = SecurityConfig::default();
    assert!(!sc.secret_scanning);
    assert!(!sc.secret_scanning_ai_detection);
    assert!(!sc.push_protection);
    assert!(!sc.dependabot_alerts);
    assert!(!sc.dependabot_security_updates);
    assert!(!sc.codeql_advanced_setup);
}

#[test]
fn serde_security_config_defaults_to_true() {
    // When deserialized with missing fields, serde uses default_true
    let sc: SecurityConfig = toml::from_str("").unwrap();
    assert!(sc.secret_scanning);
    assert!(sc.secret_scanning_ai_detection);
    assert!(sc.push_protection);
    assert!(sc.dependabot_alerts);
    assert!(sc.dependabot_security_updates);
    assert!(!sc.codeql_advanced_setup); // this one defaults false
}

#[test]
fn rulesets_config_defaults() {
    let rc = RulesetsConfig::default();
    assert!(rc.branch_protection.is_none());
}

#[test]
fn rulesets_config_serde_defaults() {
    let rc: RulesetsConfig = toml::from_str("").unwrap();
    assert!(rc.branch_protection.is_none());
}

#[test]
fn ruleset_branch_protection_serde_defaults() {
    let rbp: RulesetBranchProtection = toml::from_str("").unwrap();
    assert!(rbp.enabled);
    assert!(rbp.name.is_none());
    assert_eq!(rbp.enforcement, "active");
    assert_eq!(rbp.required_approvals, 1);
    assert!(!rbp.dismiss_stale_reviews);
    assert!(!rbp.require_code_owner_reviews);
    assert!(rbp.required_status_checks.is_empty());
    assert!(!rbp.require_linear_history);
    assert!(!rbp.block_force_pushes);
    assert!(!rbp.block_deletions);
    assert!(rbp.bypass_teams.is_empty());
}

#[test]
fn ruleset_branch_protection_custom_values() {
    let toml_str = r#"
        enabled = true
        name = "Custom Rules"
        enforcement = "evaluate"
        required_approvals = 2
        dismiss_stale_reviews = true
        require_code_owner_reviews = true
        required_status_checks = ["ci", "lint"]
        require_linear_history = true
        block_force_pushes = true
        block_deletions = true
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    assert!(rbp.enabled);
    assert_eq!(rbp.name.as_deref(), Some("Custom Rules"));
    assert_eq!(rbp.enforcement, "evaluate");
    assert_eq!(rbp.required_approvals, 2);
    assert!(rbp.dismiss_stale_reviews);
    assert!(rbp.require_code_owner_reviews);
    assert_eq!(rbp.required_status_checks, vec!["ci", "lint"]);
    assert!(rbp.require_linear_history);
    assert!(rbp.block_force_pushes);
    assert!(rbp.block_deletions);
}

#[test]
fn team_access_empty_default() {
    let toml_str = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    assert!(m.systems[0].teams.is_empty());
}

#[test]
fn team_access_parsing() {
    let toml_str = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
        teams = [
            { slug = "developers", permission = "push" },
            { slug = "devops", permission = "admin" },
        ]
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    assert_eq!(m.systems[0].teams.len(), 2);
    assert_eq!(m.systems[0].teams[0].slug, "developers");
    assert_eq!(m.systems[0].teams[0].permission, "push");
    assert_eq!(m.systems[0].teams[1].slug, "devops");
    assert_eq!(m.systems[0].teams[1].permission, "admin");
}

#[test]
fn manifest_with_rulesets_and_teams() {
    let toml_str = r#"
        [org]
        name = "org"

        [rulesets.branch_protection]
        enabled = true
        enforcement = "active"
        required_approvals = 1
        block_force_pushes = true

        [[systems]]
        id = "be"
        name = "Backend"
        teams = [
            { slug = "dev", permission = "push" },
        ]
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    let bp = m.rulesets.branch_protection.as_ref().unwrap();
    assert!(bp.enabled);
    assert_eq!(bp.enforcement, "active");
    assert_eq!(bp.required_approvals, 1);
    assert!(bp.block_force_pushes);
    assert_eq!(m.systems[0].teams.len(), 1);
}

#[test]
fn ruleset_bypass_teams_parsing() {
    let toml_str = r#"
        enabled = true
        bypass_teams = ["team-owners", "release-managers"]
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    assert_eq!(rbp.bypass_teams.len(), 2);
    assert_eq!(rbp.bypass_teams[0].slug(), "team-owners");
    assert_eq!(rbp.bypass_teams[0].bypass_mode(), "always");
    assert_eq!(rbp.bypass_teams[1].slug(), "release-managers");
    assert_eq!(rbp.bypass_teams[1].bypass_mode(), "always");
}

#[test]
fn ruleset_bypass_teams_detailed_parsing() {
    let toml_str = r#"
        enabled = true
        bypass_teams = [{ slug = "team-owners", bypass_mode = "pull_request" }]
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    assert_eq!(rbp.bypass_teams.len(), 1);
    assert_eq!(rbp.bypass_teams[0].slug(), "team-owners");
    assert_eq!(rbp.bypass_teams[0].bypass_mode(), "pull_request");
}

#[test]
fn ruleset_bypass_teams_detailed_default_mode() {
    let toml_str = r#"
        enabled = true
        bypass_teams = [{ slug = "team-owners" }]
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    assert_eq!(rbp.bypass_teams.len(), 1);
    assert_eq!(rbp.bypass_teams[0].slug(), "team-owners");
    assert_eq!(rbp.bypass_teams[0].bypass_mode(), "always");
}

#[test]
fn ruleset_bypass_teams_mixed_simple_and_detailed() {
    let toml_str = r#"
        enabled = true
        bypass_teams = ["simple-team", { slug = "detailed-team", bypass_mode = "pull_request" }]
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    assert_eq!(rbp.bypass_teams.len(), 2);
    assert_eq!(rbp.bypass_teams[0].slug(), "simple-team");
    assert_eq!(rbp.bypass_teams[0].bypass_mode(), "always");
    assert_eq!(rbp.bypass_teams[1].slug(), "detailed-team");
    assert_eq!(rbp.bypass_teams[1].bypass_mode(), "pull_request");
}

#[test]
fn manifest_with_bypass_teams() {
    let toml_str = r#"
        [org]
        name = "org"

        [rulesets.branch_protection]
        enabled = true
        required_approvals = 1
        bypass_teams = ["global-owners"]
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    let bp = m.rulesets.branch_protection.as_ref().unwrap();
    assert_eq!(bp.bypass_teams.len(), 1);
    assert_eq!(bp.bypass_teams[0].slug(), "global-owners");
}

#[test]
fn per_system_rulesets_override_bypass_teams() {
    let toml_str = r#"
        [org]
        name = "org"

        [rulesets.branch_protection]
        enabled = true
        required_approvals = 2
        dismiss_stale_reviews = true
        bypass_teams = ["global-owners"]

        [[systems]]
        id = "be"
        name = "Backend"

        [systems.rulesets.branch_protection]
        bypass_teams = ["backend-owners"]
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    let merged = m.rulesets_branch_protection_for_system("be").unwrap();
    // bypass_teams overridden by system
    assert_eq!(merged.bypass_teams.len(), 1);
    assert_eq!(merged.bypass_teams[0].slug(), "backend-owners");
    // other fields fall back to global
    assert_eq!(merged.required_approvals, 2);
    assert!(merged.dismiss_stale_reviews);
    assert!(merged.enabled);
}

#[test]
fn per_system_rulesets_override_multiple_fields() {
    let toml_str = r#"
        [org]
        name = "org"

        [rulesets.branch_protection]
        enabled = true
        required_approvals = 1
        block_force_pushes = true

        [[systems]]
        id = "fe"
        name = "Frontend"

        [systems.rulesets.branch_protection]
        required_approvals = 3
        bypass_teams = ["fe-owners"]
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    let merged = m.rulesets_branch_protection_for_system("fe").unwrap();
    assert_eq!(merged.required_approvals, 3);
    assert_eq!(merged.bypass_teams.len(), 1);
    assert_eq!(merged.bypass_teams[0].slug(), "fe-owners");
    // falls back to global
    assert!(merged.block_force_pushes);
}

#[test]
fn per_system_rulesets_falls_back_to_global_when_no_override() {
    let toml_str = r#"
        [org]
        name = "org"

        [rulesets.branch_protection]
        enabled = true
        required_approvals = 2
        bypass_teams = ["global-owners"]

        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    let config = m.rulesets_branch_protection_for_system("be").unwrap();
    assert_eq!(config.required_approvals, 2);
    assert_eq!(config.bypass_teams.len(), 1);
    assert_eq!(config.bypass_teams[0].slug(), "global-owners");
}

#[test]
fn per_system_rulesets_none_when_no_global() {
    let toml_str = r#"
        [org]
        name = "org"

        [[systems]]
        id = "be"
        name = "Backend"

        [systems.rulesets.branch_protection]
        bypass_teams = ["be-owners"]
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    // No global rulesets.branch_protection, so returns None
    assert!(m.rulesets_branch_protection_for_system("be").is_none());
}

#[test]
fn merge_with_all_none_returns_base() {
    let base = RulesetBranchProtection {
        enabled: true,
        name: Some("Base".to_string()),
        enforcement: "active".to_string(),
        required_approvals: 2,
        dismiss_stale_reviews: true,
        require_code_owner_reviews: true,
        required_status_checks: vec!["ci".to_string()],
        require_linear_history: true,
        block_force_pushes: true,
        block_deletions: true,
        bypass_teams: vec![BypassTeam::Simple("global".to_string())],
        overrides: vec![],
    };
    let over = RulesetBranchProtectionOverride::default();
    let merged = base.merge_with(&over);
    assert_eq!(merged, base);
}

#[test]
fn repo_override_pattern_matching() {
    let toml_str = r#"
        enabled = true
        required_approvals = 2
        block_force_pushes = true
        bypass_teams = ["default-admins"]

        [[overrides]]
        repo_patterns = ["*-operations", "*-system"]
        block_force_pushes = false
        bypass_teams = [{ slug = "ops-admins", bypass_mode = "always" }]
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    assert_eq!(rbp.overrides.len(), 1);
    assert_eq!(
        rbp.overrides[0].repo_patterns,
        vec!["*-operations", "*-system"]
    );
}

#[test]
fn for_repo_returns_override_for_matching_repo() {
    let toml_str = r#"
        enabled = true
        required_approvals = 2
        block_force_pushes = true
        bypass_teams = ["default-admins"]

        [[overrides]]
        repo_patterns = ["*-operations", "*-system"]
        block_force_pushes = false
        bypass_teams = [{ slug = "ops-admins", bypass_mode = "always" }]
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    let resolved = rbp.for_repo("my-service-operations");
    assert!(!resolved.block_force_pushes);
    assert_eq!(resolved.bypass_teams.len(), 1);
    assert_eq!(resolved.bypass_teams[0].slug(), "ops-admins");
    assert_eq!(resolved.required_approvals, 2); // falls back to base
    assert!(resolved.overrides.is_empty()); // overrides not carried over
}

#[test]
fn for_repo_returns_base_for_non_matching_repo() {
    let toml_str = r#"
        enabled = true
        required_approvals = 2
        block_force_pushes = true
        bypass_teams = ["default-admins"]

        [[overrides]]
        repo_patterns = ["*-operations", "*-system"]
        block_force_pushes = false
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    let resolved = rbp.for_repo("my-service-api");
    assert!(resolved.block_force_pushes);
    assert_eq!(resolved.bypass_teams.len(), 1);
    assert_eq!(resolved.bypass_teams[0].slug(), "default-admins");
    assert!(resolved.overrides.is_empty());
}

#[test]
fn for_repo_first_override_wins() {
    let toml_str = r#"
        enabled = true
        required_approvals = 1

        [[overrides]]
        repo_patterns = ["*-operations"]
        required_approvals = 3

        [[overrides]]
        repo_patterns = ["*-operations", "*-system"]
        required_approvals = 5
    "#;
    let rbp: RulesetBranchProtection = toml::from_str(toml_str).unwrap();
    let resolved = rbp.for_repo("my-service-operations");
    // First override matches, so required_approvals = 3
    assert_eq!(resolved.required_approvals, 3);
}

#[test]
fn full_manifest_with_repo_overrides() {
    let toml_str = r#"
        [org]
        name = "org"

        [rulesets.branch_protection]
        enabled = true
        required_approvals = 1
        dismiss_stale_reviews = true
        block_force_pushes = true
        bypass_teams = [{ slug = "default-admins", bypass_mode = "always" }]

        [[rulesets.branch_protection.overrides]]
        repo_patterns = ["*-operations", "*-system"]
        block_force_pushes = false
        bypass_teams = [{ slug = "ops-admins", bypass_mode = "always" }]

        [[systems]]
        id = "acme"
        name = "Party Registry"

        [systems.rulesets.branch_protection]
        bypass_teams = [{ slug = "party-owners", bypass_mode = "pull_request" }]

        [[systems.rulesets.branch_protection.overrides]]
        repo_patterns = ["*-operations"]
        bypass_teams = [{ slug = "party-owners", bypass_mode = "always" }]
    "#;
    let m: Manifest = toml::from_str(toml_str).unwrap();
    let config = m.rulesets_branch_protection_for_system("acme").unwrap();

    // system override replaces bypass_teams
    assert_eq!(config.bypass_teams.len(), 1);
    assert_eq!(config.bypass_teams[0].slug(), "party-owners");
    assert_eq!(config.bypass_teams[0].bypass_mode(), "pull_request");

    // system override replaces overrides too
    assert_eq!(config.overrides.len(), 1);
    assert_eq!(config.overrides[0].repo_patterns, vec!["*-operations"]);

    // for_repo on operations repo uses system-level override
    let ops_config = config.for_repo("acme-operations");
    assert_eq!(ops_config.bypass_teams.len(), 1);
    assert_eq!(ops_config.bypass_teams[0].slug(), "party-owners");
    assert_eq!(ops_config.bypass_teams[0].bypass_mode(), "always");

    // for_repo on non-operations repo uses base system config
    let app_config = config.for_repo("acme-api");
    assert_eq!(app_config.bypass_teams.len(), 1);
    assert_eq!(app_config.bypass_teams[0].slug(), "party-owners");
    assert_eq!(app_config.bypass_teams[0].bypass_mode(), "pull_request");
}

#[test]
fn system_for_repo_matches_exact() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.system_for_repo("be"), Some("be"));
}

#[test]
fn system_for_repo_matches_prefix_with_dash() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.system_for_repo("be-api"), Some("be"));
    assert_eq!(m.system_for_repo("be-frontend"), Some("be"));
}

#[test]
fn system_for_repo_rejects_partial_prefix() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    // "backend" starts with "be" but NOT at a boundary (no dash separator)
    assert_eq!(m.system_for_repo("backend"), None);
    assert_eq!(m.system_for_repo("bear-service"), None);
}

#[test]
fn system_for_repo_picks_longest_match() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "s07"
        name = "All S07"
        [[systems]]
        id = "s07411"
        name = "Party Management"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.system_for_repo("s07411-api"), Some("s07411"));
    assert_eq!(m.system_for_repo("s07-other"), Some("s07"));
}

#[test]
fn system_for_repo_returns_none_when_no_match() {
    let toml = r#"
        [org]
        name = "org"
        [[systems]]
        id = "be"
        name = "Backend"
    "#;
    let m: Manifest = toml::from_str(toml).unwrap();
    assert_eq!(m.system_for_repo("unrelated-repo"), None);
}

fn sample_manifest_for_v2() -> Manifest {
    Manifest {
        org: OrgConfig {
            name: "acme".to_owned(),
        },
        source: Some(SourceConfig {
            repository: "acme/reference".to_owned(),
        }),
        security: SecurityConfig {
            secret_scanning: true,
            secret_scanning_ai_detection: true,
            push_protection: true,
            dependabot_alerts: true,
            dependabot_security_updates: true,
            codeql_advanced_setup: false,
        },
        repository: Some(RepositorySettingsConfig {
            has_issues: Some(true),
            has_pull_requests: Some(true),
            pull_request_creation_policy: Some("all".to_owned()),
            has_sponsorships_enabled: Some(false),
            issue_creation_policy: Some("all".to_owned()),
            use_squash_pr_title_as_default: Some(true),
            topics: Some(vec!["managed".to_owned()]),
            ..RepositorySettingsConfig::default()
        }),
        file_delivery: FileDeliveryConfig {
            branch: "chore/ward-sync".to_owned(),
            reviewers: Vec::new(),
            commit_message_prefix: "chore: ".to_owned(),
        },
        branch_protection: BranchProtectionConfig::default(),
        rulesets: RulesetsConfig {
            branch_protection: None,
            repository: vec![RepositoryRulesetConfig {
                name: "Protect main".to_owned(),
                target: "branch".to_owned(),
                enforcement: "active".to_owned(),
                conditions_json: None,
                rules: Vec::new(),
                bypass_actors: vec![RulesetBypassActorConfig {
                    actor_type: "Team".to_owned(),
                    actor_id: Some(42),
                    team_slug: Some("platform".to_owned()),
                    bypass_mode: "always".to_owned(),
                }],
            }],
        },
        systems: vec![SystemConfig {
            id: "reference".to_owned(),
            name: "Reference".to_owned(),
            match_prefix: false,
            exclude: Vec::new(),
            repos: vec!["reference".to_owned()],
            security: None,
            teams: vec![TeamAccess {
                slug: "platform".to_owned(),
                permission: "push".to_owned(),
            }],
            rulesets: None,
        }],
        files: vec![ManagedFile {
            path: ".github/workflows/ci.yml".to_owned(),
            content: "name: CI\n".to_owned(),
        }],
        v2: ManifestV2State::default(),
    }
}

#[test]
fn manifest_v2_foundation_from_legacy_manifest_sets_safe_category_policies() {
    let manifest = sample_manifest_for_v2();

    let document = manifest.to_document_v2();

    assert_eq!(document.v2.schema.as_ref().unwrap().version, 2);
    assert_eq!(
        document
            .v2
            .provenance
            .as_ref()
            .map(|provenance| provenance.repository.as_str()),
        Some("acme/reference")
    );
    assert_eq!(
        document.v2.schema.as_ref().map(|schema| schema.version),
        Some(2)
    );
    assert_eq!(
        document
            .v2
            .categories
            .security
            .as_ref()
            .map(|category| category.policy.disposition),
        Some(ManagementDisposition::Managed)
    );
    assert_eq!(
        document
            .v2
            .categories
            .access
            .as_ref()
            .map(|category| category.policy.disposition),
        Some(ManagementDisposition::Managed)
    );
    assert_eq!(
        document
            .v2
            .categories
            .access
            .as_ref()
            .map(|category| category.teams.as_slice()),
        Some(
            [TeamAccess {
                slug: "platform".to_owned(),
                permission: "push".to_owned(),
            }]
            .as_slice()
        )
    );
    assert!(document.systems[0].teams.is_empty());
    assert_eq!(
        document
            .v2
            .categories
            .actions
            .as_ref()
            .map(|category| category.policy.disposition),
        Some(ManagementDisposition::Observe)
    );
    assert_eq!(
        document
            .v2
            .categories
            .actions
            .as_ref()
            .map(|category| category.policy.sensitive),
        Some(true)
    );
    assert_eq!(
        document
            .v2
            .categories
            .rulesets
            .as_ref()
            .unwrap()
            .repository_rulesets[0]
            .bypass_actors[0]
            .actor,
        ActorReference::Team {
            slug: "platform".to_owned(),
        }
    );
    assert_eq!(
        document.v2.categories.files.as_ref().unwrap().entries[0].encoding,
        FileEncoding::Utf8
    );
    assert_eq!(
        document
            .v2
            .categories
            .repository
            .as_ref()
            .and_then(|category| category.settings.as_ref())
            .and_then(|settings| settings.has_issues),
        Some(true)
    );
    assert_eq!(document.v2.coverage.len(), 0);
}

#[test]
fn manifest_v2_serde_defaults_keep_new_flags_safe() {
    let toml = r#"
        [schema]
        version = 2

        [org]
        name = "acme"

        [categories.actions]
    "#;

    let document: ManifestDocumentV2 = toml::from_str(toml).unwrap();
    let actions = document.v2.categories.actions.as_ref().unwrap();

    assert_eq!(actions.policy.disposition, ManagementDisposition::Observe);
    assert!(!actions.policy.prune);
    assert!(!actions.policy.sensitive);
    assert!(actions.secrets.is_empty());
    assert!(
        actions
            .settings
            .as_ref()
            .is_none_or(|settings| settings.oidc_subject_claim_include_keys.is_empty())
    );
}

#[test]
fn manifest_v2_does_not_flatten_different_system_team_grants() {
    let mut manifest = sample_manifest_for_v2();
    let mut second = manifest.systems[0].clone();
    second.id = "other".to_owned();
    second.name = "Other".to_owned();
    second.repos = vec!["other".to_owned()];
    second.teams = vec![TeamAccess {
        slug: "other-team".to_owned(),
        permission: "admin".to_owned(),
    }];
    manifest.systems.push(second);

    let document = manifest.to_document_v2();

    assert!(document.v2.categories.access.is_none());
    assert_eq!(document.systems[0].teams[0].slug, "platform");
    assert_eq!(document.systems[1].teams[0].slug, "other-team");
}

#[test]
fn manifest_v2_round_trips_legacy_semantics_through_load() {
    let rendered = sample_manifest_for_v2().to_document_v2().render().unwrap();
    let test_dir = std::path::Path::new("target/config-tests");
    std::fs::create_dir_all(test_dir).unwrap();
    let path = test_dir.join("manifest-v2-round-trip.toml");
    std::fs::write(&path, rendered).unwrap();

    let loaded = Manifest::load(path.to_str()).unwrap();

    std::fs::remove_file(&path).unwrap();

    assert_eq!(loaded.source.as_ref().unwrap().repository, "acme/reference");
    assert_eq!(loaded.systems[0].repos, vec!["reference"]);
    assert_eq!(
        loaded.rulesets.repository[0].bypass_actors[0]
            .team_slug
            .as_deref(),
        Some("platform")
    );
    assert_eq!(loaded.files[0].path, ".github/workflows/ci.yml");
    assert_eq!(loaded.v2_schema().map(|schema| schema.version), Some(2));
    assert!(loaded.v2.categories.actions.as_ref().is_some());
}

fn full_manifest_with_v2_state() -> Manifest {
    let mut manifest = sample_manifest_for_v2();
    manifest.v2 = ManifestV2State {
        schema: Some(ManifestSchema::v2()),
        provenance: Some(ManifestProvenance {
            repository: "acme/reference".to_owned(),
            default_branch: Some("main".to_owned()),
            repository_node_id: Some("R_kgDOExample".to_owned()),
            default_branch_head_oid: Some("abc123".to_owned()),
        }),
        categories: ManifestCategories {
            security: Some(SecurityCategoryV2 {
                policy: CategoryPolicy {
                    disposition: ManagementDisposition::Reference,
                    prune: false,
                    sensitive: true,
                },
                settings: Some(SecurityConfig {
                    secret_scanning: true,
                    secret_scanning_ai_detection: true,
                    push_protection: true,
                    dependabot_alerts: true,
                    dependabot_security_updates: true,
                    codeql_advanced_setup: false,
                }),
                advanced_security: Some(true),
                code_security: Some(true),
                dependabot_alerts: Some(true),
                dependabot_security_updates: Some(true),
                secret_scanning: Some(true),
                secret_scanning_push_protection: Some(true),
                secret_scanning_validity_checks: Some(false),
                secret_scanning_non_provider_patterns: Some(true),
                secret_scanning_ai_detection: Some(true),
                secret_scanning_delegated_alert_dismissal: Some(true),
                secret_scanning_delegated_bypass: Some(true),
                secret_scanning_delegated_alert_dismissal_options: Some(
                    SecurityReviewerOptionsConfigV2 {
                        reviewers: vec![SecurityReviewerConfigV2 {
                            actor: ActorReference::Team {
                                slug: "security-team".to_owned(),
                            },
                            mode: Some("always".to_owned()),
                        }],
                    },
                ),
                secret_scanning_delegated_bypass_options: Some(SecurityReviewerOptionsConfigV2 {
                    reviewers: vec![SecurityReviewerConfigV2 {
                        actor: ActorReference::Role {
                            name: "security_managers".to_owned(),
                        },
                        mode: Some("always".to_owned()),
                    }],
                }),
                private_vulnerability_reporting: Some(true),
                codeql_default_setup: Some(CodeqlDefaultSetupConfig {
                    state: Some("configured".to_owned()),
                    languages: vec!["rust".to_owned(), "javascript".to_owned()],
                    query_suite: Some("security-extended".to_owned()),
                    runner_type: Some("labeled".to_owned()),
                    runner_label: Some("ubuntu-latest".to_owned()),
                    threat_model: Some("remote_and_local".to_owned()),
                }),
                configuration_reference: Some(ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::CodeSecurityConfiguration,
                    name: "baseline".to_owned(),
                }),
                delegated_alert_dismissal_reviewers: vec![ActorReference::Team {
                    slug: "security-team".to_owned(),
                }],
                delegated_bypass_reviewers: vec![ActorReference::Role {
                    name: "security_managers".to_owned(),
                }],
                references: vec![ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::CodeSecurityConfiguration,
                    name: "org-baseline".to_owned(),
                }],
            }),
            repository: Some(RepositoryCategoryV2 {
                policy: CategoryPolicy::managed(),
                settings: manifest.repository.clone(),
                metadata: Some(RepositoryMetadataConfig {
                    description: Some("Reference service".to_owned()),
                    homepage: Some("https://example.test".to_owned()),
                    default_branch: Some("main".to_owned()),
                    visibility: Some("private".to_owned()),
                    archived: Some(false),
                    is_template: Some(false),
                    allow_forking: Some(false),
                }),
                custom_properties: vec![CustomPropertyValueConfig {
                    property_name: "system".to_owned(),
                    value: serde_json::json!(["party", "billing"]),
                }],
                immutable_releases: Some(ImmutableReleasesConfig {
                    enabled: Some(true),
                    enforced_by_owner: Some(true),
                }),
                references: vec![ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::Role,
                    name: "maintainer".to_owned(),
                }],
            }),
            branch_protection: Some(BranchProtectionCategoryV2 {
                policy: CategoryPolicy::managed(),
                default_branch: Some(BranchProtectionConfig {
                    enabled: true,
                    required_approvals: 2,
                    dismiss_stale_reviews: true,
                    require_code_owner_reviews: true,
                    require_status_checks: true,
                    strict_status_checks: true,
                    enforce_admins: true,
                    required_linear_history: true,
                    allow_force_pushes: false,
                    allow_deletions: false,
                }),
                default_branch_detailed: Some(DetailedBranchProtectionConfigV2 {
                    protection: BranchProtectionConfig {
                        enabled: true,
                        required_approvals: 2,
                        dismiss_stale_reviews: true,
                        require_code_owner_reviews: true,
                        require_status_checks: true,
                        strict_status_checks: true,
                        enforce_admins: true,
                        required_linear_history: true,
                        allow_force_pushes: false,
                        allow_deletions: false,
                    },
                    status_check_contexts: vec!["ci".to_owned(), "lint".to_owned()],
                    status_checks: vec![
                        BranchStatusCheckConfigV2 {
                            context: "ci".to_owned(),
                            app_id: Some(17),
                            app_slug: None,
                        },
                        BranchStatusCheckConfigV2 {
                            context: "lint".to_owned(),
                            app_id: None,
                            app_slug: Some("github-actions".to_owned()),
                        },
                    ],
                    push_restrictions: vec![ActorReference::Team {
                        slug: "platform".to_owned(),
                    }],
                    dismissal_restrictions: vec![ActorReference::User {
                        login: "alice".to_owned(),
                    }],
                    pull_request_bypass_allowances: vec![ActorReference::App {
                        slug: "release-bot".to_owned(),
                    }],
                    require_last_push_approval: Some(true),
                    block_creations: Some(true),
                    required_reviewers: Some(serde_json::json!({
                        "users": ["octocat"],
                        "teams": ["platform"],
                    })),
                    require_conversation_resolution: Some(true),
                    require_signed_commits: Some(true),
                    lock_branch: Some(false),
                    allow_fork_syncing: Some(false),
                }),
                protected_branches: vec![ProtectedBranchConfig {
                    name: "release/*".to_owned(),
                    protection: BranchProtectionConfig {
                        enabled: true,
                        required_approvals: 1,
                        dismiss_stale_reviews: false,
                        require_code_owner_reviews: true,
                        require_status_checks: true,
                        strict_status_checks: false,
                        enforce_admins: true,
                        required_linear_history: true,
                        allow_force_pushes: false,
                        allow_deletions: false,
                    },
                    status_check_contexts: vec!["ci".to_owned(), "lint".to_owned()],
                    status_checks: vec![
                        BranchStatusCheckConfigV2 {
                            context: "ci".to_owned(),
                            app_id: Some(17),
                            app_slug: None,
                        },
                        BranchStatusCheckConfigV2 {
                            context: "lint".to_owned(),
                            app_id: None,
                            app_slug: Some("github-actions".to_owned()),
                        },
                    ],
                    push_restrictions: vec![ActorReference::Team {
                        slug: "release-engineering".to_owned(),
                    }],
                    dismissal_restrictions: vec![ActorReference::User {
                        login: "alice".to_owned(),
                    }],
                    pull_request_bypass_allowances: vec![ActorReference::Role {
                        name: "maintain".to_owned(),
                    }],
                    require_last_push_approval: Some(true),
                    block_creations: Some(false),
                    required_reviewers: Some(serde_json::json!({
                        "users": ["octocat"],
                        "teams": ["release-engineering"],
                    })),
                    require_conversation_resolution: Some(true),
                    require_signed_commits: Some(true),
                    lock_branch: Some(false),
                    allow_fork_syncing: Some(false),
                }],
            }),
            rulesets: Some(RulesetsCategoryV2 {
                policy: CategoryPolicy::managed(),
                references: vec![RulesetReferenceV2 {
                    name: "Org baseline".to_owned(),
                    target: "branch".to_owned(),
                    enforcement: "active".to_owned(),
                    source_type: "Organization".to_owned(),
                    source: "acme".to_owned(),
                }],
                repository_rulesets: vec![RepositoryRulesetV2 {
                    name: "Protect main".to_owned(),
                    target: "branch".to_owned(),
                    enforcement: "active".to_owned(),
                    conditions_json: Some(
                        r#"{"ref_name":{"include":["~DEFAULT_BRANCH"],"exclude":[]}}"#.to_owned(),
                    ),
                    rules: vec![RepositoryRuleConfig {
                        rule_type: "deletion".to_owned(),
                        parameters_json: None,
                    }],
                    bypass_actors: vec![RulesetBypassActorV2 {
                        actor: ActorReference::Team {
                            slug: "platform".to_owned(),
                        },
                        bypass_mode: "always".to_owned(),
                    }],
                }],
            }),
            files: Some(FilesCategoryV2 {
                policy: CategoryPolicy {
                    disposition: ManagementDisposition::Managed,
                    prune: true,
                    sensitive: false,
                },
                include: vec![".github/**".to_owned(), "CODEOWNERS".to_owned()],
                exclude: vec![".github/generated/**".to_owned()],
                entries: vec![
                    ManagedFileV2 {
                        path: ".github/workflows/ci.yml".to_owned(),
                        content: "name: CI\n".to_owned(),
                        encoding: FileEncoding::Utf8,
                        mode: "100644".to_owned(),
                        source_sha: Some("deadbeef".to_owned()),
                    },
                    ManagedFileV2 {
                        path: ".github/logo.png".to_owned(),
                        content: "iVBORw0KGgo=".to_owned(),
                        encoding: FileEncoding::Base64,
                        mode: "100755".to_owned(),
                        source_sha: Some("feedface".to_owned()),
                    },
                ],
            }),
            actions: Some(ActionsCategoryV2 {
                policy: CategoryPolicy::observe_sensitive(),
                settings: Some(ActionsSettingsConfig {
                    enabled: Some(true),
                    allowed_actions: Some("selected".to_owned()),
                    selected_actions: vec!["actions/checkout@v4".to_owned()],
                    allow_github_owned_actions: Some(true),
                    allow_verified_creator_actions: Some(false),
                    requires_pinned_actions: Some(true),
                    default_workflow_permissions: Some("read".to_owned()),
                    can_approve_pull_request_reviews: Some(false),
                    artifact_retention_days: Some(30),
                    log_retention_days: Some(14),
                    private_fork_workflows_enabled: Some(false),
                    private_fork_workflow_approval: Some("maintainer".to_owned()),
                    send_write_tokens_to_workflows: Some(false),
                    send_secrets_and_variables: Some(false),
                    require_approval_for_fork_pr_workflows: Some(true),
                    fork_pull_request_workflows_enabled: Some(true),
                    fork_pull_request_contributor_approval: Some(
                        "first_time_contributors".to_owned(),
                    ),
                    workflow_access_level: Some("organization".to_owned()),
                    oidc_subject_claim_template: Some("repo:${{ github.repository }}".to_owned()),
                    oidc_use_default: Some(false),
                    oidc_use_immutable_subject: Some(true),
                    oidc_subject_claim_include_keys: vec![
                        "repository".to_owned(),
                        "repository_owner".to_owned(),
                    ],
                    cache_retention_limit_days: Some(7),
                    cache_storage_limit_gb: Some(10),
                }),
                variables: vec![NamedValueConfig {
                    name: "RUST_LOG".to_owned(),
                    value: "info".to_owned(),
                }],
                secrets: vec![SecretPlaceholderConfig {
                    name: "REGISTRY_TOKEN".to_owned(),
                    value_from: ExternalValueReference::Env {
                        key: "WARD_REGISTRY_TOKEN".to_owned(),
                    },
                }],
                dependabot_secrets: vec![SecretPlaceholderConfig {
                    name: "DEPENDABOT_TOKEN".to_owned(),
                    value_from: ExternalValueReference::Env {
                        key: "WARD_DEPENDABOT_TOKEN".to_owned(),
                    },
                }],
                codespaces_secrets: vec![SecretPlaceholderConfig {
                    name: "CODESPACES_TOKEN".to_owned(),
                    value_from: ExternalValueReference::Env {
                        key: "WARD_CODESPACES_TOKEN".to_owned(),
                    },
                }],
                workflows: vec![WorkflowStateConfig {
                    path: ".github/workflows/ci.yml".to_owned(),
                    enabled: Some(true),
                }],
                references: vec![ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::RunnerGroup,
                    name: "shared-linux".to_owned(),
                }],
            }),
            environments: Some(EnvironmentsCategoryV2 {
                policy: CategoryPolicy::observe_sensitive(),
                entries: vec![EnvironmentConfigV2 {
                    name: "production".to_owned(),
                    wait_timer_minutes: Some(30),
                    prevent_self_review: Some(true),
                    deployment_policy: Some(EnvironmentDeploymentPolicyConfig {
                        protected_branches: Some(true),
                        custom_branch_policies: Some(true),
                        branch_patterns: vec!["main".to_owned()],
                        tag_patterns: vec!["v*".to_owned()],
                    }),
                    reviewers: vec![EnvironmentReviewerConfig {
                        actor: ActorReference::Team {
                            slug: "platform".to_owned(),
                        },
                    }],
                    protection_apps: vec![ReferencedResourceConfig {
                        resource_type: ReferencedResourceType::ProtectionRule,
                        name: "change-freeze".to_owned(),
                    }],
                    variables: vec![NamedValueConfig {
                        name: "SPRING_PROFILES_ACTIVE".to_owned(),
                        value: "prod".to_owned(),
                    }],
                    secrets: vec![SecretPlaceholderConfig {
                        name: "DB_PASSWORD".to_owned(),
                        value_from: ExternalValueReference::Manual {
                            hint: Some("Stored in KeyVault".to_owned()),
                        },
                    }],
                }],
            }),
            access: Some(RepositoryAccessCategoryV2 {
                policy: CategoryPolicy::observe_sensitive(),
                teams: vec![TeamAccess {
                    slug: "platform".to_owned(),
                    permission: "maintain".to_owned(),
                }],
                collaborators: vec![CollaboratorAccessConfig {
                    actor: ActorReference::User {
                        login: "octocat".to_owned(),
                    },
                    permission: "push".to_owned(),
                }],
                references: vec![ReferencedResourceConfig {
                    resource_type: ReferencedResourceType::Team,
                    name: "org-admins".to_owned(),
                }],
            }),
            integrations: Some(RepositoryIntegrationsCategoryV2 {
                policy: CategoryPolicy::observe_sensitive(),
                webhooks: vec![WebhookConfigV2 {
                    url: "https://hooks.example.test/events".to_owned(),
                    url_from: None,
                    active: Some(true),
                    events: vec!["push".to_owned(), "pull_request".to_owned()],
                    content_type: Some("json".to_owned()),
                    insecure_ssl: Some(false),
                    secret: Some(ExternalValueReference::Env {
                        key: "WARD_WEBHOOK_SECRET".to_owned(),
                    }),
                }],
                deploy_keys: vec![DeployKeyConfigV2 {
                    title: "readonly".to_owned(),
                    read_only: Some(true),
                    fingerprint: Some("aa:bb:cc".to_owned()),
                    replacement_key: Some(ExternalValueReference::Manual {
                        hint: Some("Generate a new SSH key".to_owned()),
                    }),
                }],
                pages: Some(PagesConfigV2 {
                    build_type: Some("workflow".to_owned()),
                    source_branch: Some("gh-pages".to_owned()),
                    source_path: Some("/".to_owned()),
                    cname: Some("ward.example.test".to_owned()),
                    https_enforced: Some(true),
                }),
                autolinks: vec![AutolinkConfigV2 {
                    key_prefix: "WARD-".to_owned(),
                    url_template: "https://tracker.example.test/WARD-<num>".to_owned(),
                    is_alphanumeric: None,
                }],
                labels: vec![LabelConfigV2 {
                    name: "ward".to_owned(),
                    color: Some("0052cc".to_owned()),
                    description: Some("Managed by Ward".to_owned()),
                    default: Some(false),
                }],
            }),
        },
        coverage: vec![
            CoverageEntry {
                category: ManifestCategoryName::Actions,
                endpoint: "GET /repos/{owner}/{repo}/actions/secrets".to_owned(),
                outcome: CoverageOutcome::Redacted,
                reason: Some("GitHub does not return secret values".to_owned()),
                required_permission: Some("repo".to_owned()),
            },
            CoverageEntry {
                category: ManifestCategoryName::Security,
                endpoint: "GET /orgs/{org}/code-security/configurations".to_owned(),
                outcome: CoverageOutcome::PermissionDenied,
                reason: Some("Missing admin:org".to_owned()),
                required_permission: Some("admin:org".to_owned()),
            },
        ],
    };
    manifest
}

#[test]
fn manifest_v2_preserves_every_category_policy_coverage_and_provenance() {
    let manifest = full_manifest_with_v2_state();
    let rendered = manifest.to_document_v2().render().unwrap();
    let test_dir = std::path::Path::new("target/config-tests");
    std::fs::create_dir_all(test_dir).unwrap();
    let path = test_dir.join("manifest-v2-full-round-trip.toml");
    std::fs::write(&path, &rendered).unwrap();

    let loaded = Manifest::load(path.to_str()).unwrap();
    let rerendered = loaded.to_document_v2().render().unwrap();

    std::fs::remove_file(&path).unwrap();

    assert_eq!(rendered, rerendered);
    assert_eq!(
        loaded.v2.provenance.as_ref(),
        manifest.v2.provenance.as_ref()
    );
    assert_eq!(loaded.v2_categories(), manifest.v2_categories());
    assert_eq!(loaded.v2.coverage, manifest.v2.coverage);
    assert_eq!(loaded.v2_schema(), manifest.v2_schema());
    assert_eq!(
        loaded
            .v2
            .categories
            .actions
            .as_ref()
            .and_then(|category| category.settings.as_ref())
            .and_then(|settings| settings.artifact_retention_days),
        Some(30)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .repository
            .as_ref()
            .and_then(|category| category.settings.as_ref())
            .and_then(|settings| settings.has_pull_requests),
        Some(true)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .repository
            .as_ref()
            .and_then(|category| category.settings.as_ref())
            .and_then(|settings| settings.use_squash_pr_title_as_default),
        Some(true)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .repository
            .as_ref()
            .unwrap()
            .custom_properties[0]
            .value,
        serde_json::json!(["party", "billing"])
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .actions
            .as_ref()
            .and_then(|category| category.settings.as_ref())
            .and_then(|settings| settings.send_write_tokens_to_workflows),
        Some(false)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .actions
            .as_ref()
            .and_then(|category| category.settings.as_ref())
            .and_then(|settings| settings.oidc_use_default),
        Some(false)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .actions
            .as_ref()
            .map(|category| category.dependabot_secrets.len()),
        Some(1)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .actions
            .as_ref()
            .map(|category| category.codespaces_secrets.len()),
        Some(1)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .security
            .as_ref()
            .and_then(|category| category.secret_scanning_delegated_bypass),
        Some(true)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .security
            .as_ref()
            .and_then(|category| {
                category
                    .secret_scanning_delegated_bypass_options
                    .as_ref()
                    .map(|options| options.reviewers[0].mode.as_deref())
            })
            .flatten(),
        Some("always")
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .branch_protection
            .as_ref()
            .and_then(|category| category.default_branch_detailed.as_ref())
            .map(|branch| branch.status_checks.len()),
        Some(2)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .branch_protection
            .as_ref()
            .unwrap()
            .protected_branches[0]
            .required_reviewers
            .as_ref(),
        Some(&serde_json::json!({
            "users": ["octocat"],
            "teams": ["release-engineering"],
        }))
    );
    assert_eq!(
        loaded
            .v2_categories()
            .rulesets
            .as_ref()
            .map(|category| category.references[0].source_type.as_str()),
        Some("Organization")
    );
    assert_eq!(
        loaded.v2.categories.environments.as_ref().unwrap().entries[0].protection_apps[0].name,
        "change-freeze"
    );
    assert_eq!(
        loaded.v2.categories.access.as_ref().unwrap().collaborators[0].permission,
        "push"
    );
    assert_eq!(
        loaded.v2.categories.integrations.as_ref().unwrap().labels[0].default,
        Some(false)
    );
    assert_eq!(
        loaded
            .v2
            .categories
            .integrations
            .as_ref()
            .unwrap()
            .pages
            .as_ref()
            .and_then(|pages| pages.cname.as_deref()),
        Some("ward.example.test")
    );
}

#[test]
fn manifest_v2_golden_serialization_is_explicit_and_readable() {
    let document = full_manifest_with_v2_state().to_document_v2();

    let rendered = document.render().unwrap();
    assert!(rendered.contains("[schema]"));
    assert!(rendered.contains("[provenance]"));
    assert!(rendered.contains("[categories.repository.policy]"));
    assert!(rendered.contains("[categories.branch_protection.policy]"));
    assert!(rendered.contains("[categories.branch_protection.default_branch_detailed]"));
    assert!(rendered.contains("[categories.actions.policy]"));
    assert!(rendered.contains("advanced_security = true"));
    assert!(rendered.contains("dependabot_secrets"));
    assert!(rendered.contains("codespaces_secrets"));
    assert!(rendered.contains("secret_scanning_delegated_bypass_options"));
    assert!(rendered.contains("[[categories.rulesets.references]]"));
    assert!(rendered.contains("[categories.environments.policy]"));
    assert!(rendered.contains("[categories.integrations.policy]"));
}

#[test]
fn manifest_v2_actions_new_fields_default_safely_and_round_trip() {
    let toml = r#"
        [schema]
        version = 2

        [org]
        name = "acme"

        [categories.actions]

        [categories.actions.settings]
        enabled = true
    "#;

    let document: ManifestDocumentV2 = toml::from_str(toml).unwrap();
    let actions = document.v2.categories.actions.as_ref().unwrap();
    let settings = actions.settings.as_ref().unwrap();

    assert_eq!(settings.send_write_tokens_to_workflows, None);
    assert_eq!(settings.send_secrets_and_variables, None);
    assert_eq!(settings.require_approval_for_fork_pr_workflows, None);
    assert_eq!(settings.oidc_use_default, None);
    assert_eq!(settings.oidc_use_immutable_subject, None);
    assert!(actions.dependabot_secrets.is_empty());
    assert!(actions.codespaces_secrets.is_empty());

    let rendered = document.render().unwrap();
    let reparsed: ManifestDocumentV2 = toml::from_str(&rendered).unwrap();
    let reparsed_actions = reparsed.v2.categories.actions.as_ref().unwrap();

    assert!(reparsed_actions.dependabot_secrets.is_empty());
    assert!(reparsed_actions.codespaces_secrets.is_empty());
    assert_eq!(
        reparsed_actions
            .settings
            .as_ref()
            .unwrap()
            .oidc_use_immutable_subject,
        None
    );
}

#[test]
fn manifest_v2_repository_settings_and_custom_property_arrays_round_trip() {
    let toml = r#"
        [schema]
        version = 2

        [org]
        name = "acme"

        [repository]
        has_pull_requests = true
        pull_request_creation_policy = "all"
        has_sponsorships_enabled = false
        issue_creation_policy = "all"
        use_squash_pr_title_as_default = true

        [[categories.repository.custom_properties]]
        property_name = "owners"
        value = ["platform", "billing"]

        [[categories.integrations.labels]]
        name = "bug"
        color = "d73a4a"
        description = "Bug"
        default = true
    "#;

    let test_dir = std::path::Path::new("target/config-tests");
    std::fs::create_dir_all(test_dir).unwrap();
    let path = test_dir.join("manifest-v2-repository-array-round-trip.toml");
    std::fs::write(&path, toml).unwrap();

    let manifest = Manifest::load(path.to_str()).unwrap();

    assert_eq!(
        manifest
            .repository
            .as_ref()
            .and_then(|settings| settings.has_pull_requests),
        Some(true)
    );
    assert_eq!(
        manifest
            .v2
            .categories
            .repository
            .as_ref()
            .unwrap()
            .custom_properties[0]
            .value,
        serde_json::json!(["platform", "billing"])
    );
    assert_eq!(
        manifest.v2.categories.integrations.as_ref().unwrap().labels[0].default,
        Some(true)
    );

    std::fs::remove_file(path).unwrap();
}

#[test]
fn manifest_v2_string_custom_property_and_new_repository_fields_are_backward_compatible() {
    let toml = r#"
        [schema]
        version = 2

        [org]
        name = "acme"

        [repository]
        has_issues = true

        [[categories.repository.custom_properties]]
        property_name = "team"
        value = "party"
    "#;

    let document: ManifestDocumentV2 = toml::from_str(toml).unwrap();
    let settings = document.repository.as_ref().unwrap();

    assert_eq!(settings.has_issues, Some(true));
    assert_eq!(settings.has_pull_requests, None);
    assert_eq!(settings.pull_request_creation_policy, None);
    assert_eq!(settings.has_sponsorships_enabled, None);
    assert_eq!(settings.issue_creation_policy, None);
    assert_eq!(settings.use_squash_pr_title_as_default, None);
    assert_eq!(
        document
            .v2
            .categories
            .repository
            .as_ref()
            .unwrap()
            .custom_properties[0]
            .value,
        serde_json::json!("party")
    );

    let rendered = document.render().unwrap();
    assert!(rendered.contains("value = \"party\""));
}

#[test]
fn manifest_v2_security_rules_and_branch_additions_default_safely() {
    let toml = r#"
        [schema]
        version = 2

        [org]
        name = "acme"

        [categories.security]

        [categories.branch_protection]

        [[categories.branch_protection.protected_branches]]
        name = "release/*"

        [categories.rulesets]
    "#;

    let document: ManifestDocumentV2 = toml::from_str(toml).unwrap();
    let security = document.v2.categories.security.as_ref().unwrap();
    let branch_protection = document.v2.categories.branch_protection.as_ref().unwrap();
    let rulesets = document.v2.categories.rulesets.as_ref().unwrap();

    assert_eq!(security.advanced_security, None);
    assert_eq!(security.code_security, None);
    assert_eq!(security.secret_scanning_validity_checks, None);
    assert_eq!(security.secret_scanning_delegated_alert_dismissal, None);
    assert!(security.secret_scanning_delegated_bypass_options.is_none());
    assert!(branch_protection.default_branch_detailed.is_none());
    assert!(
        branch_protection.protected_branches[0]
            .status_checks
            .is_empty()
    );
    assert_eq!(
        branch_protection.protected_branches[0].require_last_push_approval,
        None
    );
    assert_eq!(
        branch_protection.protected_branches[0].block_creations,
        None
    );
    assert_eq!(
        branch_protection.protected_branches[0].required_reviewers,
        None
    );
    assert!(rulesets.references.is_empty());
}

#[test]
fn checked_in_example_manifest_is_valid_v2_configuration() {
    let manifest: Manifest = toml::from_str(include_str!("../../../ward.example.toml")).unwrap();

    assert_eq!(manifest.v2_schema().map(|schema| schema.version), Some(2));
    assert_eq!(manifest.org.name, "my-github-org");
    assert_eq!(
        manifest
            .v2_categories()
            .files
            .as_ref()
            .map(|category| category.entries.len()),
        Some(1)
    );
    assert_eq!(
        manifest
            .v2
            .categories
            .actions
            .as_ref()
            .and_then(|category| category.secrets.first())
            .map(|secret| secret.name.as_str()),
        Some("DEPLOY_TOKEN")
    );
}
