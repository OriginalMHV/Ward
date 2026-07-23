use super::*;

#[test]
fn minimal_manifest_uses_current_schema_and_empty_categories() {
    let manifest: Manifest = toml::from_str(
        r#"
        [org]
        name = "acme"
        "#,
    )
    .unwrap();

    assert_eq!(manifest.org.name, "acme");
    assert_eq!(manifest.schema, ManifestSchema::current());
    assert!(manifest.categories.is_empty());
    assert!(manifest.systems.is_empty());
}

#[test]
fn canonical_manifest_round_trips_without_compatibility_fields() {
    let source = r#"
        [org]
        name = "acme"

        [schema]
        version = 2

        [provenance]
        repository = "acme/reference"
        default_branch = "main"

        [file_delivery]
        branch = "chore/ward-sync"
        reviewers = ["alice"]
        commit_message_prefix = "chore: "

        [[systems]]
        id = "backend"
        name = "Backend"
        repos = ["special-repo"]

        [categories.repository.policy]
        disposition = "managed"

        [categories.repository.settings]
        has_issues = true
        allow_squash_merge = true
        topics = ["ward"]

        [categories.security]
        secret_scanning = true
        secret_scanning_push_protection = true

        [categories.security.policy]
        disposition = "managed"
        sensitive = true

        [categories.access.policy]
        disposition = "managed"
        sensitive = true

        [[categories.access.teams]]
        slug = "developers"
        permission = "push"
    "#;

    let document: ManifestDocument = toml::from_str(source).unwrap();
    let rendered = document.render().unwrap();
    let reparsed: ManifestDocument = toml::from_str(&rendered).unwrap();

    assert_eq!(reparsed, document);
    assert!(!rendered.contains("\n[security]"));
    assert!(!rendered.contains("\n[repository]"));
    assert!(!rendered.contains("\n[source]"));
}

#[test]
fn legacy_top_level_sections_are_rejected() {
    let error = toml::from_str::<Manifest>(
        r#"
        [org]
        name = "acme"

        [security]
        secret_scanning = true
        "#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn source_section_is_rejected_in_favor_of_provenance() {
    let error = toml::from_str::<Manifest>(
        r#"
        [org]
        name = "acme"

        [source]
        repository = "acme/reference"
        "#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn load_rejects_unsupported_schema_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ward.toml");
    std::fs::write(
        &path,
        r#"
        [org]
        name = "acme"

        [schema]
        version = 99
        "#,
    )
    .unwrap();

    let error = Manifest::load(path.to_str()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Unsupported Ward manifest schema version 99")
    );
}

#[test]
fn system_targeting_supports_explicit_repos_and_longest_prefix() {
    let manifest: Manifest = toml::from_str(
        r#"
        [org]
        name = "acme"

        [[systems]]
        id = "svc"
        name = "Services"

        [[systems]]
        id = "svc-api"
        name = "API"

        [[systems]]
        id = "manual"
        name = "Manual"
        match_prefix = false
        repos = ["one-off"]
        "#,
    )
    .unwrap();

    assert_eq!(manifest.system_for_repo("svc-worker"), Some("svc"));
    assert_eq!(manifest.system_for_repo("svc-api-users"), Some("svc-api"));
    assert_eq!(manifest.system_for_repo("one-off"), Some("manual"));
    assert_eq!(manifest.system_for_repo("manual-other"), None);
}

#[test]
fn system_categories_replace_only_the_configured_global_categories() {
    let manifest: Manifest = toml::from_str(
        r#"
        [org]
        name = "acme"

        [[systems]]
        id = "backend"
        name = "Backend"

        [systems.categories.access.policy]
        disposition = "managed"

        [[systems.categories.access.teams]]
        slug = "backend-admins"
        permission = "admin"

        [categories.security]
        secret_scanning = true

        [categories.access.policy]
        disposition = "managed"

        [[categories.access.teams]]
        slug = "developers"
        permission = "push"
        "#,
    )
    .unwrap();

    let backend = manifest.categories_for_repo("backend-api");
    let unrelated = manifest.categories_for_repo("frontend");

    assert_eq!(
        backend.access.unwrap().teams[0].slug,
        "backend-admins".to_owned()
    );
    assert!(backend.security.unwrap().secret_scanning.unwrap());
    assert_eq!(
        unrelated.access.unwrap().teams[0].slug,
        "developers".to_owned()
    );
}

#[test]
fn omitted_system_category_inherits_global_desired_state() {
    let manifest: Manifest = toml::from_str(
        r#"
        [org]
        name = "acme"

        [[systems]]
        id = "backend"
        name = "Backend"

        [systems.categories.security]
        secret_scanning = false

        [categories.repository.settings]
        has_issues = true

        [categories.security]
        secret_scanning = true
        "#,
    )
    .unwrap();

    let categories = manifest.categories_for_repo("backend-api");
    assert_eq!(
        categories.repository.unwrap().settings.unwrap().has_issues,
        Some(true)
    );
    assert_eq!(categories.security.unwrap().secret_scanning, Some(false));
}

#[test]
fn file_delivery_defaults_are_stable() {
    let manifest: Manifest = toml::from_str(
        r#"
        [org]
        name = "acme"
        "#,
    )
    .unwrap();

    assert_eq!(manifest.file_delivery.branch, "chore/ward-sync");
    assert_eq!(manifest.file_delivery.commit_message_prefix, "chore: ");
    assert!(manifest.file_delivery.reviewers.is_empty());
}

#[test]
fn checked_in_example_is_a_valid_canonical_manifest() {
    let manifest: Manifest = toml::from_str(include_str!("../../../ward.example.toml")).unwrap();

    assert_eq!(manifest.schema, ManifestSchema::current());
    assert_eq!(manifest.org.name, "my-github-org");
    assert_eq!(
        manifest
            .categories
            .files
            .as_ref()
            .map(|category| category.entries.len()),
        Some(1)
    );
    assert_eq!(
        manifest
            .categories
            .actions
            .as_ref()
            .and_then(|category| category.secrets.first())
            .map(|secret| secret.name.as_str()),
        Some("DEPLOY_TOKEN")
    );
}
