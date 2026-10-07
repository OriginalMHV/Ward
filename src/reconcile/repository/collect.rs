//! Collection of repository settings, metadata and coverage.

use super::plan::{insert_value, normalize_optional_policy};
use super::resources::{
    collect_custom_properties, collect_labels, manifest_compatible_custom_properties,
    normalize_topics,
};
use super::{CollectedGeneralState, GeneralCollectedExtensions};
use crate::config::manifest::{
    CategoryPolicy, CoverageEntry, CoverageOutcome, ImmutableReleasesConfig, ManifestCategoryName,
    RepositoryCategory, RepositoryMetadataConfig, RepositorySettingsConfig,
};
use crate::github::Client;
use crate::github::settings::{
    ClassifiedApiResponse, GraphqlRepositorySettings, ImmutableReleasesState,
    RepositoryGeneralSettings,
};
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

pub async fn collect(client: &Client, repo: &str) -> Result<CollectedGeneralState> {
    let rest = client.get_repository_general_settings(repo).await?;
    collect_with_rest(client, repo, rest).await
}

/// Collect general state from an already fetched `GET /repos/{repo}` response.
pub(super) fn collected(endpoint: &str) -> CoverageEntry {
    coverage_entry(
        ManifestCategoryName::Repository,
        endpoint,
        CoverageOutcome::Collected,
        None,
        None,
    )
}

pub async fn collect_with_rest(
    client: &Client,
    repo: &str,
    rest: RepositoryGeneralSettings,
) -> Result<CollectedGeneralState> {
    let mut coverage = unsupported_repository_settings_coverage();
    coverage.push(collected("GET /repos/{owner}/{repo}"));

    let (graphql_result, topics_result, properties_result, immutable_result, labels_result) = tokio::join!(
        client.get_repository_graphql_settings_classified(repo),
        client.get_topics_classified(repo),
        client.get_custom_property_values(repo),
        client.get_immutable_releases_state_classified(repo),
        client.list_labels_classified(repo),
    );

    let graphql = match graphql_result? {
        ClassifiedApiResponse::Success(settings) => {
            coverage.push(collected("POST /graphql repository settings"));
            Some(settings)
        }
        ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::PermissionDenied,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Unprocessable(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Conflict(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "POST /graphql repository settings",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NoContent => None,
    };

    let topics = match topics_result? {
        ClassifiedApiResponse::Success(values) => {
            coverage.push(collected("GET /repos/{owner}/{repo}/topics"));
            Some(normalize_topics(&values))
        }
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/topics",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("read".to_owned()),
            ));
            None
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/topics",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/topics",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NoContent => Some(Vec::new()),
    };

    let custom_properties = match properties_result? {
        ClassifiedApiResponse::Success(values) => {
            coverage.push(collected("GET /repos/{owner}/{repo}/properties/values"));
            collect_custom_properties(&values)
        }
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/properties/values",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("read".to_owned()),
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/properties/values",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/properties/values",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NoContent => Vec::new(),
    };

    let immutable_releases = match immutable_result? {
        ClassifiedApiResponse::Success(state) => {
            coverage.push(collected("GET /repos/{owner}/{repo}/immutable-releases"));
            Some(state)
        }
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/immutable-releases",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("admin".to_owned()),
            ));
            None
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/immutable-releases",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/immutable-releases",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            None
        }
        ClassifiedApiResponse::NoContent => Some(ImmutableReleasesState {
            enabled: false,
            enforced_by_owner: false,
        }),
    };

    let labels = match labels_result? {
        ClassifiedApiResponse::Success(labels) => {
            coverage.push(collected("GET /repos/{owner}/{repo}/labels"));
            collect_labels(labels)
        }
        ClassifiedApiResponse::Forbidden(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/labels",
                CoverageOutcome::PermissionDenied,
                Some(message),
                Some("read".to_owned()),
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NotFound(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/labels",
                CoverageOutcome::NotApplicable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::Unprocessable(message)
        | ClassifiedApiResponse::Conflict(message)
        | ClassifiedApiResponse::Other(message) => {
            coverage.push(coverage_entry(
                ManifestCategoryName::Repository,
                "GET /repos/{owner}/{repo}/labels",
                CoverageOutcome::Unavailable,
                Some(message),
                None,
            ));
            Vec::new()
        }
        ClassifiedApiResponse::NoContent => Vec::new(),
    };

    let repository = RepositoryCategory {
        policy: CategoryPolicy::managed(),
        settings: Some(build_repository_settings(
            &rest,
            topics.as_ref(),
            graphql.as_ref(),
        )?),
        metadata: Some(build_repository_metadata(&rest)?),
        custom_properties: manifest_compatible_custom_properties(&custom_properties),
        immutable_releases: immutable_releases
            .as_ref()
            .map(|state| ImmutableReleasesConfig {
                enabled: Some(state.enabled),
                enforced_by_owner: Some(state.enforced_by_owner),
            }),
        references: Vec::new(),
    };

    let labels_collected = !read_failed(&coverage, "GET /repos/{owner}/{repo}/labels");
    let custom_properties_collected =
        !read_failed(&coverage, "GET /repos/{owner}/{repo}/properties/values");

    Ok(CollectedGeneralState {
        repository,
        labels,
        custom_properties,
        coverage,
        extensions: GeneralCollectedExtensions {
            repository_id: rest.node_id,
            graphql_settings_collected: graphql.is_some(),
            labels_collected,
            custom_properties_collected,
            immutable_releases_collected: immutable_releases.is_some(),
            has_discussions_enabled: Some(
                graphql
                    .as_ref()
                    .map_or(rest.has_discussions, |value| value.has_discussions_enabled),
            ),
            has_pull_requests: Some(rest.has_pull_requests),
            pull_request_creation_policy: normalize_optional_policy(
                rest.pull_request_creation_policy.as_deref(),
            ),
            has_sponsorships_enabled: graphql.as_ref().map(|value| value.has_sponsorships_enabled),
            issue_creation_policy: graphql.as_ref().and_then(|value| {
                normalize_optional_policy(value.issue_creation_policy.as_deref())
            }),
            use_squash_pr_title_as_default: rest.use_squash_pr_title_as_default,
        },
    })
}

pub(super) fn build_repository_settings(
    rest: &RepositoryGeneralSettings,
    topics: Option<&Vec<String>>,
    graphql: Option<&GraphqlRepositorySettings>,
) -> Result<RepositorySettingsConfig> {
    let mut map = Map::new();
    insert_value(&mut map, "has_issues", json!(rest.has_issues));
    insert_value(&mut map, "has_projects", json!(rest.has_projects));
    insert_value(&mut map, "has_wiki", json!(rest.has_wiki));
    insert_value(
        &mut map,
        "has_discussions",
        json!(graphql.map_or(rest.has_discussions, |value| value.has_discussions_enabled)),
    );
    insert_value(&mut map, "has_pull_requests", json!(rest.has_pull_requests));
    if let Some(value) = normalize_optional_policy(rest.pull_request_creation_policy.as_deref()) {
        insert_value(&mut map, "pull_request_creation_policy", json!(value));
    }
    insert_value(
        &mut map,
        "allow_squash_merge",
        json!(rest.allow_squash_merge),
    );
    insert_value(
        &mut map,
        "allow_merge_commit",
        json!(rest.allow_merge_commit),
    );
    insert_value(
        &mut map,
        "allow_rebase_merge",
        json!(rest.allow_rebase_merge),
    );
    insert_value(&mut map, "allow_auto_merge", json!(rest.allow_auto_merge));
    insert_value(
        &mut map,
        "delete_branch_on_merge",
        json!(rest.delete_branch_on_merge),
    );
    insert_value(
        &mut map,
        "allow_update_branch",
        json!(rest.allow_update_branch),
    );
    if let Some(value) = rest.use_squash_pr_title_as_default {
        insert_value(&mut map, "use_squash_pr_title_as_default", json!(value));
    }
    if let Some(value) = rest.squash_merge_commit_title.as_ref() {
        insert_value(&mut map, "squash_merge_commit_title", json!(value));
    }
    if let Some(value) = rest.squash_merge_commit_message.as_ref() {
        insert_value(&mut map, "squash_merge_commit_message", json!(value));
    }
    if let Some(value) = rest.merge_commit_title.as_ref() {
        insert_value(&mut map, "merge_commit_title", json!(value));
    }
    if let Some(value) = rest.merge_commit_message.as_ref() {
        insert_value(&mut map, "merge_commit_message", json!(value));
    }
    insert_value(
        &mut map,
        "web_commit_signoff_required",
        json!(rest.web_commit_signoff_required),
    );
    if let Some(values) = topics {
        insert_value(&mut map, "topics", json!(values));
    }
    if let Some(graphql) = graphql {
        insert_value(
            &mut map,
            "has_sponsorships_enabled",
            json!(graphql.has_sponsorships_enabled),
        );
        if let Some(value) = normalize_optional_policy(graphql.issue_creation_policy.as_deref()) {
            insert_value(&mut map, "issue_creation_policy", json!(value));
        }
    }

    serde_json::from_value(Value::Object(map))
        .context("Failed to build repository settings snapshot from collected state")
}

pub(super) fn build_repository_metadata(
    rest: &RepositoryGeneralSettings,
) -> Result<RepositoryMetadataConfig> {
    let mut map = Map::new();
    insert_value(
        &mut map,
        "description",
        json!(rest.description.clone().unwrap_or_default()),
    );
    insert_value(
        &mut map,
        "homepage",
        json!(rest.homepage.clone().unwrap_or_default()),
    );
    insert_value(&mut map, "default_branch", json!(rest.default_branch));
    insert_value(&mut map, "visibility", json!(rest.visibility));
    insert_value(&mut map, "archived", json!(rest.archived));
    insert_value(&mut map, "is_template", json!(rest.is_template));
    insert_value(&mut map, "allow_forking", json!(rest.allow_forking));

    serde_json::from_value(Value::Object(map))
        .context("Failed to build repository metadata snapshot from collected state")
}

/// True when an endpoint has a coverage entry other than `Collected`, meaning its read failed.
pub(super) fn read_failed(coverage: &[CoverageEntry], endpoint: &str) -> bool {
    coverage
        .iter()
        .any(|entry| entry.endpoint == endpoint && entry.outcome != CoverageOutcome::Collected)
}

pub(super) fn unsupported_repository_settings_coverage() -> Vec<CoverageEntry> {
    [
        "commit comments",
        "LFS archives",
        "multi-ref push limit",
        "auto-close linked issues",
    ]
    .into_iter()
    .map(|setting| {
        coverage_entry(
            ManifestCategoryName::Repository,
            &format!("Repository settings UI: {setting}"),
            CoverageOutcome::Unsupported,
            Some(
                "GitHub OpenAPI and GraphQL do not expose an official mutation for this setting"
                    .to_owned(),
            ),
            None,
        )
    })
    .collect()
}

pub(super) fn coverage_entry(
    category: ManifestCategoryName,
    endpoint: &str,
    outcome: CoverageOutcome,
    reason: Option<String>,
    required_permission: Option<String>,
) -> CoverageEntry {
    CoverageEntry {
        category,
        endpoint: endpoint.to_owned(),
        outcome,
        reason,
        required_permission,
    }
}
