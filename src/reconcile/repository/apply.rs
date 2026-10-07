//! Application and verification of repository plans.

use super::collect::collect;
use super::plan::plan_with_options;
use super::resources::normalize_label_color;
use super::{
    GeneralDesiredState, GeneralPlan, GeneralPlanOptions, GeneralVerification,
    PlannedImmutableReleaseAction, PlannedLabelAction,
};
use crate::github::Client;
use crate::github::settings::{ClassifiedApiResponse, CustomPropertyValueMutation};
use anyhow::{Context, Result, bail};
use serde_json::Value;

pub async fn apply(client: &Client, plan: &GeneralPlan) -> Result<GeneralVerification> {
    if plan.has_blocked_changes() {
        bail!(
            "General settings plan for {} is blocked by {} change(s)",
            plan.repo,
            plan.blocked_changes.len()
        );
    }

    if let Some(branch) = plan
        .rest_patch
        .as_object()
        .and_then(|body| body.get("default_branch"))
        .and_then(Value::as_str)
        && !client.branch_exists(&plan.repo, branch).await?
    {
        bail!(
            "Cannot set default branch for {} to {branch}: branch does not exist",
            plan.repo
        );
    }

    if plan
        .rest_patch
        .as_object()
        .is_some_and(|body| !body.is_empty())
    {
        client.update_settings(&plan.repo, &plan.rest_patch).await?;
    }

    if let Some(graphql_patch) = plan.graphql_patch.as_ref() {
        client
            .update_repository_graphql_settings(&plan.repository_id, graphql_patch)
            .await?;
    }

    if let Some(topics) = plan.topics.as_ref() {
        client.replace_topics(&plan.repo, topics).await?;
    }

    if !plan.custom_property_updates.is_empty() {
        let updates = plan
            .custom_property_updates
            .iter()
            .cloned()
            .map(|update| CustomPropertyValueMutation {
                property_name: update.property_name,
                value: update.value,
            })
            .collect::<Vec<_>>();
        match client
            .update_custom_property_values(&plan.repo, &updates)
            .await?
        {
            ClassifiedApiResponse::Success(()) | ClassifiedApiResponse::NoContent => {}
            ClassifiedApiResponse::Forbidden(message)
            | ClassifiedApiResponse::NotFound(message)
            | ClassifiedApiResponse::Unprocessable(message)
            | ClassifiedApiResponse::Conflict(message)
            | ClassifiedApiResponse::Other(message) => {
                bail!(
                    "Failed to update custom properties for {}: {message}",
                    plan.repo
                );
            }
        }
    }

    match plan.immutable_releases.as_ref() {
        Some(PlannedImmutableReleaseAction::Enable) => {
            match client.enable_immutable_releases(&plan.repo).await? {
                ClassifiedApiResponse::Success(()) | ClassifiedApiResponse::NoContent => {}
                ClassifiedApiResponse::Forbidden(message)
                | ClassifiedApiResponse::NotFound(message)
                | ClassifiedApiResponse::Unprocessable(message)
                | ClassifiedApiResponse::Conflict(message)
                | ClassifiedApiResponse::Other(message) => {
                    bail!(
                        "Failed to enable immutable releases for {}: {message}",
                        plan.repo
                    );
                }
            }
        }
        Some(PlannedImmutableReleaseAction::Disable) => {
            match client.disable_immutable_releases(&plan.repo).await? {
                ClassifiedApiResponse::Success(()) | ClassifiedApiResponse::NoContent => {}
                ClassifiedApiResponse::Forbidden(message)
                | ClassifiedApiResponse::NotFound(message)
                | ClassifiedApiResponse::Unprocessable(message)
                | ClassifiedApiResponse::Conflict(message)
                | ClassifiedApiResponse::Other(message) => {
                    bail!(
                        "Failed to disable immutable releases for {}: {message}",
                        plan.repo
                    );
                }
            }
        }
        Some(PlannedImmutableReleaseAction::Reference) | None => {}
    }

    for action in &plan.label_actions {
        match action {
            PlannedLabelAction::Create { label } => {
                let color = label.color.as_deref().with_context(|| {
                    format!("Cannot create label {} without a color", label.name)
                })?;
                client
                    .create_label(
                        &plan.repo,
                        &label.name,
                        &normalize_label_color(color),
                        label.description.as_deref(),
                    )
                    .await?;
            }
            PlannedLabelAction::Update {
                current_name,
                label,
            } => {
                let normalized_color = label.color.as_deref().map(normalize_label_color);
                client
                    .update_label(
                        &plan.repo,
                        current_name,
                        None,
                        normalized_color.as_deref(),
                        label.description.as_deref(),
                    )
                    .await?;
            }
            PlannedLabelAction::Delete { name } => {
                client.delete_label(&plan.repo, name).await?;
            }
        }
    }

    let verification = verify_with_options(client, &plan.repo, &plan.desired, plan.options).await?;
    if !verification.compliant {
        bail!(
            "General settings verification failed for {}: {} remaining change(s), {} blocked change(s)",
            plan.repo,
            verification.remaining_changes.len(),
            verification.blocked_changes.len()
        );
    }

    Ok(verification)
}

pub async fn verify_with_options(
    client: &Client,
    repo: &str,
    desired: &GeneralDesiredState,
    options: GeneralPlanOptions,
) -> Result<GeneralVerification> {
    let current = collect(client, repo).await?;
    let planned = plan_with_options(repo, desired, &current, options);
    let compliant = !planned.has_actionable_changes() && !planned.has_blocked_changes();

    Ok(GeneralVerification {
        repo: repo.to_owned(),
        compliant,
        coverage: current.coverage,
        remaining_changes: planned.changes,
        blocked_changes: planned.blocked_changes,
    })
}
