use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Args;
use console::style;

use crate::config::manifest::{
    ActionsCategoryV2, BranchProtectionCategoryV2, CategoryPolicy, CoverageEntry, CoverageOutcome,
    EnvironmentsCategoryV2, ExternalValueReference, FileDeliveryConfig, FilesCategoryV2,
    LabelConfigV2, Manifest, ManifestCategories, ManifestCategoryName, ManifestProvenance,
    ManifestSchema, OrgConfig, RepositoryAccessCategoryV2, RepositoryIntegrationsCategoryV2,
    RulesetsCategoryV2, SecurityCategoryV2, SystemConfig,
};
use crate::github::Client;
use crate::reconcile::access_integrations::{collect_access, collect_integrations};
use crate::reconcile::actions_environments::{
    IssueSeverity, collect_actions_category, collect_environments_category,
};
use crate::reconcile::files::{FilesIssueSeverity, collect_files_category};
use crate::reconcile::general;
use crate::reconcile::security_rules::{
    ReconcileIssueSeverity, collect_branch_protection_category, collect_rulesets_category,
    collect_security_category,
};

const DEFAULT_OUTPUT: &str = "ward.toml";
const DEFAULT_BRANCH: &str = "chore/ward-sync";

#[derive(Args)]
pub struct ImportCommand {
    /// Repository to use as the configuration source (OWNER/REPO or GitHub URL)
    pub source: String,

    /// Existing target repository. Repeat for multiple targets; defaults to the source.
    #[arg(long, value_name = "OWNER/REPO")]
    target: Vec<String>,

    /// Include configuration files matching this glob. Repeatable.
    #[arg(long, value_name = "GLOB")]
    include: Vec<String>,

    /// Exclude configuration files matching this glob. Repeatable.
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Fail instead of recording permission-denied or unavailable coverage.
    #[arg(long)]
    strict: bool,

    /// Output path
    #[arg(long, default_value = DEFAULT_OUTPUT)]
    output: PathBuf,

    /// Print only the generated configuration to stdout.
    #[arg(long)]
    stdout: bool,

    /// Replace an existing output file.
    #[arg(long)]
    force: bool,

    /// Max concurrent API calls.
    #[arg(long, default_value_t = 5)]
    parallelism: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRef {
    pub owner: String,
    pub repo: String,
}

impl RepositoryRef {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

impl fmt::Display for RepositoryRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

impl FromStr for RepositoryRef {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let trimmed = value.trim().trim_end_matches('/');
        let path = if let Some(path) = trimmed.strip_prefix("https://github.com/") {
            path
        } else if let Some(path) = trimmed.strip_prefix("http://github.com/") {
            path
        } else if let Some(path) = trimmed.strip_prefix("git@github.com:") {
            path
        } else if trimmed.contains("://") {
            anyhow::bail!("Only github.com repository URLs are supported");
        } else {
            trimmed
        };

        let path = path.trim_matches('/').trim_end_matches(".git");
        let mut segments = path.split('/');
        let owner = segments.next().unwrap_or_default();
        let repo = segments.next().unwrap_or_default();

        if owner.is_empty() || repo.is_empty() || segments.next().is_some() {
            anyhow::bail!(
                "Invalid repository '{value}'. Use OWNER/REPO or https://github.com/OWNER/REPO"
            );
        }

        Ok(Self {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }
}

pub struct ImportOptions<'a> {
    pub source: &'a str,
    pub targets: &'a [String],
    pub include: &'a [String],
    pub exclude: &'a [String],
    pub strict: bool,
    pub output: &'a Path,
    pub stdout: bool,
    pub force: bool,
    pub parallelism: usize,
}

#[derive(Debug)]
struct Snapshot {
    manifest: Manifest,
    warnings: Vec<String>,
    strict_failures: Vec<String>,
    counts: SnapshotCounts,
}

#[derive(Debug, Default)]
struct SnapshotCounts {
    files: usize,
    rulesets: usize,
    inherited_rulesets: usize,
    workflows: usize,
    environments: usize,
    access_entries: usize,
    integrations: usize,
}

impl ImportCommand {
    pub async fn run(self) -> Result<()> {
        import_repository(ImportOptions {
            source: &self.source,
            targets: &self.target,
            include: &self.include,
            exclude: &self.exclude,
            strict: self.strict,
            output: &self.output,
            stdout: self.stdout,
            force: self.force,
            parallelism: self.parallelism,
        })
        .await
    }
}

pub async fn import_repository(options: ImportOptions<'_>) -> Result<()> {
    let source = RepositoryRef::from_str(options.source)?;
    ensure_output_is_available(&options)?;

    progress(
        options.stdout,
        format!(
            "{} Reading all documented repository configuration from {}...",
            style("[..]").dim(),
            style(source.to_string()).cyan().bold()
        ),
    );

    let client = Client::new(&source.owner, options.parallelism).await?;
    let source_repository = client
        .get_repo(&source.repo)
        .await
        .with_context(|| format!("Failed to read source repository {source}"))?;
    let targets = resolve_targets(&client, &source, options.targets).await?;
    let snapshot = snapshot_repository(
        &client,
        &source,
        &source_repository.default_branch,
        &targets,
        options.include,
        options.exclude,
    )
    .await?;

    if options.strict && !snapshot.strict_failures.is_empty() {
        anyhow::bail!(
            "Strict import failed because {} item(s) could not be collected:\n  - {}",
            snapshot.strict_failures.len(),
            snapshot.strict_failures.join("\n  - ")
        );
    }

    let output = render_manifest(&snapshot.manifest)?;
    if options.stdout {
        print!("{output}");
    } else {
        write_manifest(options.output, &output)?;
        println!(
            "  {} Wrote {}",
            style("[ok]").green(),
            style(options.output.display()).bold()
        );
    }

    print_summary(&snapshot, &source, &targets, options.stdout);
    Ok(())
}

fn ensure_output_is_available(options: &ImportOptions<'_>) -> Result<()> {
    if !options.stdout && options.output.exists() && !options.force {
        anyhow::bail!(
            "{} already exists. Use --force to replace it or --stdout to preview.",
            options.output.display()
        );
    }
    Ok(())
}

async fn resolve_targets(
    client: &Client,
    source: &RepositoryRef,
    requested: &[String],
) -> Result<Vec<String>> {
    let raw_targets = if requested.is_empty() {
        vec![source.repo.clone()]
    } else {
        requested.to_vec()
    };

    let mut targets = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in raw_targets {
        let target = parse_target(&raw, &source.owner)?;
        if target.owner != source.owner {
            anyhow::bail!(
                "Target {target} has a different owner. Ward imports only existing repositories under {}.",
                source.owner
            );
        }
        if !seen.insert(target.repo.clone()) {
            continue;
        }

        client.get_repo(&target.repo).await.with_context(|| {
            format!("Target repository {target} does not exist or is unreadable")
        })?;
        targets.push(target.repo);
    }

    Ok(targets)
}

fn parse_target(value: &str, default_owner: &str) -> Result<RepositoryRef> {
    let value = value.trim();
    if value.contains('/') || value.contains(':') {
        RepositoryRef::from_str(value)
    } else if value.is_empty() {
        anyhow::bail!("Target repository name cannot be empty")
    } else {
        Ok(RepositoryRef {
            owner: default_owner.to_owned(),
            repo: value.to_owned(),
        })
    }
}

async fn snapshot_repository(
    client: &Client,
    source: &RepositoryRef,
    default_branch: &str,
    targets: &[String],
    include: &[String],
    exclude: &[String],
) -> Result<Snapshot> {
    let access_seed = RepositoryAccessCategoryV2::observe_sensitive();
    let integrations_seed = RepositoryIntegrationsCategoryV2::observe_sensitive();
    let files_seed = FilesCategoryV2 {
        policy: CategoryPolicy::managed(),
        include: include.to_vec(),
        exclude: exclude.to_vec(),
        entries: Vec::new(),
    };

    let (
        general_result,
        security_result,
        rulesets_result,
        branch_protection_result,
        files_result,
        actions_result,
        environments_result,
        access_result,
        integrations_result,
        head_oid_result,
    ) = tokio::join!(
        general::collect(client, &source.repo),
        collect_security_category(client, &source.repo, None),
        collect_rulesets_category(client, &source.repo, None),
        collect_branch_protection_category(client, &source.repo, None),
        collect_files_category(
            client,
            &source.repo,
            Some(default_branch),
            Some(&files_seed)
        ),
        collect_actions_category(client, &source.repo, None),
        collect_environments_category(client, &source.repo, None),
        collect_access(client, &source.repo, &access_seed),
        collect_integrations(client, &source.repo, &integrations_seed),
        client.get_branch_head_sha(&source.repo, default_branch),
    );

    let mut coverage = Vec::new();
    let mut warnings = Vec::new();
    let mut strict_failures = Vec::new();

    let (repository_category, labels, repository_node_id) = match general_result {
        Ok(state) => {
            absorb_coverage(&mut coverage, state.coverage.clone());
            (
                state.repository,
                state
                    .labels
                    .into_iter()
                    .map(LabelConfigV2::from)
                    .collect::<Vec<_>>(),
                non_empty(state.extensions.repository_id),
            )
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Repository,
                "repository/general collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            (empty_repository_category(), Vec::new(), None)
        }
    };

    let security_category = match security_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::observe_sensitive();
            absorb_coverage(&mut coverage, state.coverage);
            record_security_issues(
                "security",
                &state.issues,
                &mut warnings,
                &mut strict_failures,
            );
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Security,
                "security collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            SecurityCategoryV2::observe_sensitive()
        }
    };

    let rulesets_category = match rulesets_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::observe_sensitive();
            absorb_coverage(&mut coverage, state.coverage);
            record_security_issues(
                "rulesets",
                &state.issues,
                &mut warnings,
                &mut strict_failures,
            );
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Rulesets,
                "rulesets collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            observe_rulesets_category()
        }
    };

    let branch_protection_category = match branch_protection_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::observe_sensitive();
            absorb_coverage(&mut coverage, state.coverage);
            record_security_issues(
                "branch protection",
                &state.issues,
                &mut warnings,
                &mut strict_failures,
            );
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::BranchProtection,
                "branch-protection collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            observe_branch_protection_category()
        }
    };

    let files_category = match files_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::managed();
            absorb_coverage(&mut coverage, state.coverage);
            for issue in state.issues {
                warnings.push(format!("files: {}", issue.message));
                if issue.severity == FilesIssueSeverity::Blocker {
                    strict_failures.push(format!("files: {}", issue.message));
                }
            }
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Files,
                "configuration-files collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            files_seed
        }
    };

    let actions_category = match actions_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::observe_sensitive();
            normalize_actions_placeholders(&mut state.category);
            absorb_coverage(&mut coverage, state.coverage);
            record_actions_issues(
                "actions",
                &state.issues,
                &mut warnings,
                &mut strict_failures,
            );
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Actions,
                "Actions collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            ActionsCategoryV2::observe_sensitive()
        }
    };

    let environments_category = match environments_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::observe_sensitive();
            normalize_environment_placeholders(&mut state.category);
            absorb_coverage(&mut coverage, state.coverage);
            record_actions_issues(
                "environments",
                &state.issues,
                &mut warnings,
                &mut strict_failures,
            );
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Environments,
                "environments collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            EnvironmentsCategoryV2::observe_sensitive()
        }
    };

    let access_category = match access_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::observe_sensitive();
            absorb_coverage(&mut coverage, state.coverage);
            record_actions_issues("access", &state.issues, &mut warnings, &mut strict_failures);
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Access,
                "access collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            RepositoryAccessCategoryV2::observe_sensitive()
        }
    };

    let mut integrations_category = match integrations_result {
        Ok(mut state) => {
            state.category.policy = CategoryPolicy::observe_sensitive();
            normalize_integration_placeholders(&mut state.category);
            absorb_coverage(&mut coverage, state.coverage);
            record_actions_issues(
                "integrations",
                &state.issues,
                &mut warnings,
                &mut strict_failures,
            );
            state.category
        }
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Integrations,
                "integrations collector",
                error,
                &mut coverage,
                &mut warnings,
            );
            RepositoryIntegrationsCategoryV2::observe_sensitive()
        }
    };
    integrations_category.labels = labels;

    let default_branch_head_oid = match head_oid_result {
        Ok(oid) => Some(oid),
        Err(error) => {
            record_collector_failure(
                ManifestCategoryName::Repository,
                "default-branch head",
                error,
                &mut coverage,
                &mut warnings,
            );
            None
        }
    };

    coverage.sort_by(|left, right| {
        category_rank(left.category)
            .cmp(&category_rank(right.category))
            .then_with(|| left.endpoint.cmp(&right.endpoint))
    });
    warnings.sort();
    warnings.dedup();

    strict_failures.extend(coverage.iter().filter_map(strict_coverage_failure));
    strict_failures.sort();
    strict_failures.dedup();

    let categories = ManifestCategories {
        security: Some(security_category.clone()),
        repository: Some(repository_category.clone()),
        branch_protection: Some(branch_protection_category.clone()),
        rulesets: Some(rulesets_category.clone()),
        files: Some(files_category.clone()),
        actions: Some(actions_category.clone()),
        environments: Some(environments_category.clone()),
        access: Some(access_category.clone()),
        integrations: Some(integrations_category.clone()),
    };

    let manifest = Manifest {
        org: OrgConfig {
            name: source.owner.clone(),
        },
        file_delivery: FileDeliveryConfig {
            branch: DEFAULT_BRANCH.to_owned(),
            reviewers: Vec::new(),
            commit_message_prefix: "chore: ".to_owned(),
        },
        systems: vec![SystemConfig {
            id: source.repo.clone(),
            name: format!("Imported from {source}"),
            match_prefix: false,
            exclude: Vec::new(),
            repos: targets.to_vec(),
            categories: ManifestCategories::default(),
        }],
        schema: ManifestSchema::current(),
        provenance: Some(ManifestProvenance {
            repository: source.full_name(),
            default_branch: Some(default_branch.to_owned()),
            repository_node_id,
            default_branch_head_oid,
        }),
        categories,
        coverage,
    };

    let counts = SnapshotCounts {
        files: files_category.entries.len(),
        rulesets: rulesets_category.repository_rulesets.len(),
        inherited_rulesets: rulesets_category.references.len(),
        workflows: actions_category.workflows.len(),
        environments: environments_category.entries.len(),
        access_entries: access_category.teams.len()
            + access_category.collaborators.len()
            + access_category.references.len(),
        integrations: integrations_category.webhooks.len()
            + integrations_category.deploy_keys.len()
            + integrations_category.autolinks.len()
            + usize::from(integrations_category.pages.is_some())
            + integrations_category.labels.len(),
    };

    Ok(Snapshot {
        manifest,
        warnings,
        strict_failures,
        counts,
    })
}

fn empty_repository_category() -> crate::config::manifest::RepositoryCategoryV2 {
    crate::config::manifest::RepositoryCategoryV2 {
        policy: CategoryPolicy::managed(),
        settings: None,
        metadata: None,
        custom_properties: Vec::new(),
        immutable_releases: None,
        references: Vec::new(),
    }
}

fn observe_rulesets_category() -> RulesetsCategoryV2 {
    RulesetsCategoryV2 {
        policy: CategoryPolicy::observe_sensitive(),
        references: Vec::new(),
        repository_rulesets: Vec::new(),
    }
}

fn observe_branch_protection_category() -> BranchProtectionCategoryV2 {
    BranchProtectionCategoryV2 {
        policy: CategoryPolicy::observe_sensitive(),
        default_branch: None,
        default_branch_detailed: None,
        protected_branches: Vec::new(),
    }
}

fn absorb_coverage(target: &mut Vec<CoverageEntry>, entries: Vec<CoverageEntry>) {
    target.extend(entries);
}

fn record_collector_failure(
    category: ManifestCategoryName,
    collector: &str,
    error: anyhow::Error,
    coverage: &mut Vec<CoverageEntry>,
    warnings: &mut Vec<String>,
) {
    let message = error.to_string();
    coverage.push(CoverageEntry {
        category,
        endpoint: collector.to_owned(),
        outcome: CoverageOutcome::Unavailable,
        reason: Some(message.clone()),
        required_permission: None,
    });
    warnings.push(format!("{collector}: {message}"));
}

fn record_security_issues(
    category: &str,
    issues: &[crate::reconcile::security_rules::ReconcileIssue],
    warnings: &mut Vec<String>,
    strict_failures: &mut Vec<String>,
) {
    for issue in issues {
        let message = format!("{category}: {}", issue.message);
        warnings.push(message.clone());
        if issue.severity == ReconcileIssueSeverity::Blocker {
            strict_failures.push(message);
        }
    }
}

fn record_actions_issues(
    category: &str,
    issues: &[crate::reconcile::actions_environments::ReconcileIssue],
    warnings: &mut Vec<String>,
    strict_failures: &mut Vec<String>,
) {
    for issue in issues {
        let message = format!("{category}: {}", issue.message);
        warnings.push(message.clone());
        if issue.severity == IssueSeverity::Blocker {
            strict_failures.push(message);
        }
    }
}

fn strict_coverage_failure(entry: &CoverageEntry) -> Option<String> {
    matches!(
        entry.outcome,
        CoverageOutcome::PermissionDenied | CoverageOutcome::Unavailable
    )
    .then(|| {
        format!(
            "{:?} {}: {}",
            entry.category,
            entry.endpoint,
            entry
                .reason
                .as_deref()
                .unwrap_or("state could not be collected")
        )
    })
}

fn category_rank(category: ManifestCategoryName) -> u8 {
    match category {
        ManifestCategoryName::Repository => 0,
        ManifestCategoryName::Files => 1,
        ManifestCategoryName::Security => 2,
        ManifestCategoryName::Actions => 3,
        ManifestCategoryName::Environments => 4,
        ManifestCategoryName::Access => 5,
        ManifestCategoryName::Integrations => 6,
        ManifestCategoryName::Rulesets => 7,
        ManifestCategoryName::BranchProtection => 8,
    }
}

fn normalize_actions_placeholders(category: &mut ActionsCategoryV2) {
    normalize_secret_placeholders("WARD_ACTIONS_SECRET", &mut category.secrets);
    normalize_secret_placeholders("WARD_DEPENDABOT_SECRET", &mut category.dependabot_secrets);
    normalize_secret_placeholders("WARD_CODESPACES_SECRET", &mut category.codespaces_secrets);
}

fn normalize_environment_placeholders(category: &mut EnvironmentsCategoryV2) {
    for environment in &mut category.entries {
        let prefix = format!("WARD_ENV_{}_SECRET", env_component(&environment.name));
        normalize_secret_placeholders(&prefix, &mut environment.secrets);
    }
}

fn normalize_integration_placeholders(category: &mut RepositoryIntegrationsCategoryV2) {
    for (index, webhook) in category.webhooks.iter_mut().enumerate() {
        if let Some(url_from) = webhook.url_from.as_mut() {
            normalize_external_value(url_from, format!("WARD_WEBHOOK_URL_{}", index + 1));
        }
        if let Some(secret) = webhook.secret.as_mut() {
            normalize_external_value(secret, format!("WARD_WEBHOOK_SECRET_{}", index + 1));
        }
    }

    for (index, key) in category.deploy_keys.iter_mut().enumerate() {
        if let Some(replacement_key) = key.replacement_key.as_mut() {
            normalize_external_value(
                replacement_key,
                format!(
                    "WARD_DEPLOY_KEY_{}_{}",
                    env_component(&key.title),
                    index + 1
                ),
            );
        }
    }
}

fn normalize_secret_placeholders(
    prefix: &str,
    secrets: &mut [crate::config::manifest::SecretPlaceholderConfig],
) {
    for secret in secrets {
        normalize_external_value(
            &mut secret.value_from,
            format!("{prefix}_{}", env_component(&secret.name)),
        );
    }
}

fn normalize_external_value(reference: &mut ExternalValueReference, key: String) {
    if matches!(reference, ExternalValueReference::Manual { .. }) {
        *reference = ExternalValueReference::Env { key };
    }
}

fn env_component(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut previous_was_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_uppercase());
            previous_was_separator = false;
        } else if !previous_was_separator && !result.is_empty() {
            result.push('_');
            previous_was_separator = true;
        }
    }

    let result = result.trim_matches('_');
    if result.is_empty() {
        "VALUE".to_owned()
    } else {
        result.to_owned()
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn render_manifest(manifest: &Manifest) -> Result<String> {
    let body = manifest
        .to_document()
        .render()
        .context("Failed to serialize ward.toml")?;
    let source = manifest
        .provenance
        .as_ref()
        .map(|source| source.repository.as_str())
        .unwrap_or("unknown");
    Ok(format!(
        "# Ward configuration imported from {source}\n\
         # Import is read-only. Run `ward plan` before `ward apply`.\n\
         # Sensitive categories are observe-only until their policy is changed to managed.\n\n\
         {body}"
    ))
}

fn write_manifest(output: &Path, content: &str) -> Result<()> {
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let temporary = output.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temporary, content)
        .with_context(|| format!("Failed to write {}", temporary.display()))?;
    std::fs::rename(&temporary, output)
        .with_context(|| format!("Failed to replace {}", output.display()))?;
    Ok(())
}

fn progress(stdout: bool, message: String) {
    if stdout {
        eprintln!("\n  {message}");
    } else {
        println!("\n  {message}");
    }
}

fn print_summary(snapshot: &Snapshot, source: &RepositoryRef, targets: &[String], stdout: bool) {
    let mut lines = vec![
        String::new(),
        format!(
            "  {} Imported {} documented repository categories from {}",
            style("[ok]").green(),
            9,
            source
        ),
        format!(
            "    files={}, rulesets={} (+{} inherited refs), workflows={}, environments={}",
            snapshot.counts.files,
            snapshot.counts.rulesets,
            snapshot.counts.inherited_rulesets,
            snapshot.counts.workflows,
            snapshot.counts.environments
        ),
        format!(
            "    access entries={}, integrations/labels={}",
            snapshot.counts.access_entries, snapshot.counts.integrations
        ),
        format!("    targets={}", targets.join(", ")),
    ];

    if !snapshot.warnings.is_empty() {
        lines.push(format!(
            "  {} {} partial/unsupported item(s) are recorded in [coverage].",
            style("note:").yellow(),
            snapshot.warnings.len()
        ));
    }
    lines.push(
        "  Safe defaults: repository metadata and configuration files are managed; sensitive categories are observe-only."
            .to_owned(),
    );

    for line in lines {
        if stdout {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{
        ActionsSettingsConfig, FileEncoding, ManagedFileV2, ManagementDisposition,
        RepositoryCategoryV2, RepositoryMetadataConfig, RepositorySettingsConfig,
        SecretPlaceholderConfig,
    };
    use crate::reconcile::general::GeneralLabel;

    #[test]
    fn parses_supported_repository_references() {
        for value in [
            "acme/platform",
            "https://github.com/acme/platform.git",
            "git@github.com:acme/platform.git",
        ] {
            let repository = RepositoryRef::from_str(value).unwrap();
            assert_eq!(repository.full_name(), "acme/platform");
        }
    }

    #[test]
    fn rejects_unsupported_repository_references() {
        assert!(RepositoryRef::from_str("https://example.com/acme/platform").is_err());
        assert!(RepositoryRef::from_str("acme/platform/issues").is_err());
        assert!(RepositoryRef::from_str("platform").is_err());
    }

    #[test]
    fn target_without_owner_uses_source_owner() {
        assert_eq!(
            parse_target("service", "acme").unwrap(),
            RepositoryRef {
                owner: "acme".to_owned(),
                repo: "service".to_owned(),
            }
        );
    }

    #[test]
    fn external_placeholders_become_deterministic_environment_variables() {
        let mut actions = ActionsCategoryV2 {
            settings: Some(ActionsSettingsConfig::default()),
            secrets: vec![SecretPlaceholderConfig {
                name: "API_TOKEN".to_owned(),
                value_from: ExternalValueReference::Manual { hint: None },
            }],
            ..ActionsCategoryV2::default()
        };

        normalize_actions_placeholders(&mut actions);

        assert_eq!(
            actions.secrets[0].value_from,
            ExternalValueReference::Env {
                key: "WARD_ACTIONS_SECRET_API_TOKEN".to_owned(),
            }
        );
    }

    #[test]
    fn environment_variable_components_are_shell_safe() {
        assert_eq!(env_component("production/eu-west"), "PRODUCTION_EU_WEST");
        assert_eq!(env_component("***"), "VALUE");
    }

    #[test]
    fn strict_mode_only_rejects_missing_readable_state() {
        let unsupported = CoverageEntry {
            category: ManifestCategoryName::Repository,
            endpoint: "commit-comments".to_owned(),
            outcome: CoverageOutcome::Unsupported,
            reason: None,
            required_permission: None,
        };
        let denied = CoverageEntry {
            category: ManifestCategoryName::Actions,
            endpoint: "actions/secrets".to_owned(),
            outcome: CoverageOutcome::PermissionDenied,
            reason: Some("requires Actions secrets read permission".to_owned()),
            required_permission: None,
        };

        assert!(strict_coverage_failure(&unsupported).is_none());
        assert!(strict_coverage_failure(&denied).is_some());
    }

    #[test]
    fn rendered_manifest_round_trips_binary_files() {
        let repository = RepositoryCategoryV2 {
            policy: CategoryPolicy::managed(),
            settings: Some(RepositorySettingsConfig {
                has_issues: Some(true),
                ..RepositorySettingsConfig::default()
            }),
            metadata: Some(RepositoryMetadataConfig {
                description: Some("Reference".to_owned()),
                ..RepositoryMetadataConfig::default()
            }),
            custom_properties: Vec::new(),
            immutable_releases: None,
            references: Vec::new(),
        };
        let files = FilesCategoryV2 {
            policy: CategoryPolicy::managed(),
            include: vec![".github/**".to_owned()],
            exclude: Vec::new(),
            entries: vec![ManagedFileV2 {
                path: ".github/logo.bin".to_owned(),
                content: "AAEC".to_owned(),
                encoding: FileEncoding::Base64,
                mode: "100644".to_owned(),
                source_sha: Some("abc".to_owned()),
            }],
        };
        let integrations = RepositoryIntegrationsCategoryV2 {
            policy: CategoryPolicy::observe_sensitive(),
            labels: vec![LabelConfigV2::from(GeneralLabel {
                name: "bug".to_owned(),
                color: Some("ff0000".to_owned()),
                description: None,
                default: false,
            })],
            ..RepositoryIntegrationsCategoryV2::default()
        };
        let manifest = Manifest {
            org: OrgConfig {
                name: "acme".to_owned(),
            },
            file_delivery: FileDeliveryConfig::default(),
            systems: vec![SystemConfig {
                id: "reference".to_owned(),
                name: "Reference".to_owned(),
                match_prefix: false,
                exclude: Vec::new(),
                repos: vec!["reference".to_owned()],
                categories: ManifestCategories::default(),
            }],
            schema: ManifestSchema::current(),
            provenance: Some(ManifestProvenance {
                repository: "acme/reference".to_owned(),
                default_branch: Some("main".to_owned()),
                repository_node_id: Some("R_123".to_owned()),
                default_branch_head_oid: Some("deadbeef".to_owned()),
            }),
            categories: ManifestCategories {
                repository: Some(repository),
                files: Some(files),
                integrations: Some(integrations),
                ..ManifestCategories::default()
            },
            coverage: Vec::new(),
        };

        let rendered = render_manifest(&manifest).unwrap();
        let parsed: Manifest = toml::from_str(&rendered).unwrap();

        let parsed_files = parsed.categories.files.unwrap();
        assert_eq!(parsed_files.entries[0].encoding, FileEncoding::Base64);
        assert_eq!(
            parsed.categories.integrations.unwrap().policy.disposition,
            ManagementDisposition::Observe
        );
        assert_eq!(
            parsed.provenance.unwrap().repository_node_id.as_deref(),
            Some("R_123")
        );
    }
}
