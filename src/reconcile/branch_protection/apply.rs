//! Branch protection apply and verify.

use anyhow::Result;

use crate::config::manifest::BranchProtectionCategory;
use crate::github::Client;
use crate::reconcile::common::rules_issue::ReconcileIssueSeverity;

use super::collect::*;
use super::plan::*;
use super::*;

pub async fn apply_branch_protection_plan(
    client: &Client,
    repo: &str,
    plan: &BranchProtectionPlan,
) -> Result<BranchProtectionApplyResult> {
    if let Some(issue) = plan
        .issues
        .iter()
        .find(|issue| issue.severity == ReconcileIssueSeverity::Blocker)
    {
        anyhow::bail!("Branch protection plan is blocked: {}", issue.message);
    }

    let mut applied_steps = Vec::new();
    for action in &plan.actions {
        match action {
            BranchProtectionPlanAction::Upsert { branch, desired } => {
                client
                    .update_branch_protection_detailed(repo, branch, desired)
                    .await?;
                if let Some(required) = desired.require_signed_commits {
                    client
                        .set_required_signatures(repo, branch, required)
                        .await?;
                }
                applied_steps.push(format!("upsert:{branch}"));
            }
            BranchProtectionPlanAction::Delete { branch } => {
                client.delete_branch_protection(repo, branch).await?;
                applied_steps.push(format!("delete:{branch}"));
            }
            BranchProtectionPlanAction::Unchanged { .. } => {}
        }
    }

    Ok(BranchProtectionApplyResult { applied_steps })
}

pub async fn verify_branch_protection_category(
    client: &Client,
    repo: &str,
    desired: &BranchProtectionCategory,
) -> Result<BranchProtectionVerifyResult> {
    let actual = collect_branch_protection_category(client, repo, Some(desired)).await?;
    let plan = plan_branch_protection_category(desired, &actual)?;
    let blocked = plan
        .issues
        .iter()
        .any(|issue| issue.severity == ReconcileIssueSeverity::Blocker);
    Ok(BranchProtectionVerifyResult {
        matches: !plan.has_changes() && !blocked,
        plan,
    })
}
