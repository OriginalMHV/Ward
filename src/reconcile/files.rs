//! Configuration-file snapshot and reconciliation for `FilesCategoryV2`.
//!
//! When `include` is empty, Ward observes the known root configuration registry,
//! including `.github/**`, supported CODEOWNERS locations, Renovate metadata,
//! common lint configuration, pre-commit metadata, and release metadata.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::config::manifest::{
    CategoryPolicy, CoverageEntry, CoverageOutcome, FileEncoding, FilesCategoryV2, ManagedFileV2,
    ManagementDisposition, ManifestCategoryName,
};
use crate::github::Client;
use crate::github::commits::{AtomicCommitEntry, AtomicCommitFile, CommitContent, DeleteTreeEntry};
use crate::github::contents::{
    GitEntryMode, GitObjectType, GitTreeReadStatus, validate_relative_git_path,
};

pub const KNOWN_CONFIG_INCLUDE_GLOBS: &[&str] = &[
    ".github/**",
    ".devcontainer/**",
    "devcontainer.json",
    "CODEOWNERS",
    "docs/CODEOWNERS",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "SUPPORT.md",
    "CODE_OF_CONDUCT.md",
    "GOVERNANCE.md",
    "CITATION.cff",
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    "sonar-project.properties",
    ".shellcheckrc",
    "renovate.json",
    "renovate.json5",
    ".renovaterc",
    ".renovaterc.json",
    ".commitlintrc",
    ".commitlintrc.json",
    ".commitlintrc.yaml",
    ".commitlintrc.yml",
    "commitlint.config.js",
    "commitlint.config.cjs",
    ".markdownlint*",
    ".yamllint*",
    ".pre-commit-config.yaml",
    "lefthook.yml",
    "lefthook.yaml",
    ".releaserc*",
    "release.config.js",
    "release.config.cjs",
    "release-please-config.json",
];

pub const MAX_MANAGED_BLOB_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesIssueSeverity {
    Warning,
    Blocker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesIssueKind {
    TruncatedTree,
    EmptyRepository,
    PermissionDenied,
    NotFound,
    UnsafePath,
    ExcludedDesiredPath,
    DuplicateDesiredPath,
    UnsupportedMode,
    UnknownMode,
    InvalidBase64,
    Symlink,
    Submodule,
    LfsPointer,
    Oversized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesIssue {
    pub path: Option<String>,
    pub kind: FilesIssueKind,
    pub severity: FilesIssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopedRepoFileKind {
    Managed,
    UnsupportedMode,
    Symlink,
    Submodule,
    LfsPointer,
    Oversized,
    UnsafePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedRepoFile {
    pub path: String,
    pub mode: Option<GitEntryMode>,
    pub raw_mode: String,
    pub object_type: GitObjectType,
    pub sha: String,
    pub size: Option<u64>,
    pub kind: ScopedRepoFileKind,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilesCollection {
    pub category: FilesCategoryV2,
    pub scoped_files: Vec<ScopedRepoFile>,
    pub issues: Vec<FilesIssue>,
    pub coverage: Vec<CoverageEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesPlan {
    pub upserts: Vec<ManagedFileV2>,
    pub deletions: Vec<DeleteTreeEntry>,
    pub unchanged: Vec<String>,
    pub atomic_entries: Vec<AtomicCommitEntry>,
    pub issues: Vec<FilesIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesApplyResult {
    pub commit_sha: Option<String>,
    pub entry_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesVerifyResult {
    pub matches: bool,
    pub plan: FilesPlan,
}

#[derive(Debug, Clone)]
struct FileScope {
    include: Vec<String>,
    exclude: Vec<String>,
    explicit_paths: BTreeSet<String>,
}

impl FileScope {
    fn from_category(category: Option<&FilesCategoryV2>) -> Self {
        let include = category
            .filter(|category| !category.include.is_empty())
            .map(|category| category.include.clone())
            .unwrap_or_else(|| {
                KNOWN_CONFIG_INCLUDE_GLOBS
                    .iter()
                    .map(|pattern| (*pattern).to_owned())
                    .collect()
            });
        let exclude = category
            .map(|category| category.exclude.clone())
            .unwrap_or_default();
        let explicit_paths = category
            .map(|category| {
                category
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect()
            })
            .unwrap_or_default();

        Self {
            include,
            exclude,
            explicit_paths,
        }
    }

    fn is_excluded(&self, path: &str) -> bool {
        self.exclude
            .iter()
            .any(|pattern| glob_match::glob_match(pattern, path))
    }

    fn is_selected(&self, path: &str) -> bool {
        if self.explicit_paths.contains(path) {
            return true;
        }

        self.include
            .iter()
            .any(|pattern| glob_match::glob_match(pattern, path))
            && !self.is_excluded(path)
    }
}

pub async fn collect_files_category(
    client: &Client,
    repo: &str,
    branch: Option<&str>,
    category: Option<&FilesCategoryV2>,
) -> Result<FilesCollection> {
    let scope = FileScope::from_category(category);
    let tree_read = client.read_git_tree_recursive(repo, branch).await?;

    let mut coverage = vec![coverage_entry(
        repo,
        branch,
        CoverageOutcome::Collected,
        None,
        None,
    )];

    let Some(tree) = tree_read.listing else {
        let (kind, severity, outcome) = match tree_read.status {
            GitTreeReadStatus::Available => unreachable!("available tree read without listing"),
            GitTreeReadStatus::EmptyRepository => (
                FilesIssueKind::EmptyRepository,
                FilesIssueSeverity::Blocker,
                CoverageOutcome::Unavailable,
            ),
            GitTreeReadStatus::PermissionDenied => (
                FilesIssueKind::PermissionDenied,
                FilesIssueSeverity::Blocker,
                CoverageOutcome::PermissionDenied,
            ),
            GitTreeReadStatus::NotFound => (
                FilesIssueKind::NotFound,
                FilesIssueSeverity::Blocker,
                CoverageOutcome::Unavailable,
            ),
        };
        let reason = tree_read.detail.clone();
        coverage = vec![coverage_entry(
            repo,
            branch,
            outcome,
            reason.clone(),
            (tree_read.status == GitTreeReadStatus::PermissionDenied).then_some("contents:read"),
        )];

        return Ok(FilesCollection {
            category: FilesCategoryV2 {
                policy: category
                    .map(|category| category.policy.clone())
                    .unwrap_or_else(CategoryPolicy::observe),
                include: scope.include,
                exclude: scope.exclude,
                entries: Vec::new(),
            },
            scoped_files: Vec::new(),
            issues: vec![FilesIssue {
                path: None,
                kind,
                severity,
                message: reason
                    .unwrap_or_else(|| format!("Failed to collect managed files for {repo}")),
            }],
            coverage,
            truncated: false,
        });
    };

    let mut issues = Vec::new();
    if tree.truncated {
        coverage.push(coverage_entry(
            repo,
            branch,
            CoverageOutcome::Unavailable,
            Some("Git tree response was truncated".to_owned()),
            None,
        ));
        issues.push(FilesIssue {
            path: None,
            kind: FilesIssueKind::TruncatedTree,
            severity: FilesIssueSeverity::Warning,
            message: format!(
                "Git tree listing for {repo} was truncated; prune planning will be blocked"
            ),
        });
    }

    let mut scoped_files = Vec::new();
    let mut managed_entries = Vec::new();

    for entry in tree.entries {
        if entry.object_type == GitObjectType::Tree || !scope.is_selected(&entry.path) {
            continue;
        }

        let mut scoped = ScopedRepoFile {
            path: entry.path.clone(),
            mode: entry.mode,
            raw_mode: entry.mode_display().to_owned(),
            object_type: entry.object_type,
            sha: entry.sha.clone(),
            size: entry.size,
            kind: ScopedRepoFileKind::Managed,
            bytes: None,
        };

        if let Err(error) = validate_relative_git_path(&entry.path) {
            scoped.kind = ScopedRepoFileKind::UnsafePath;
            issues.push(FilesIssue {
                path: Some(entry.path.clone()),
                kind: FilesIssueKind::UnsafePath,
                severity: FilesIssueSeverity::Warning,
                message: error.to_string(),
            });
            scoped_files.push(scoped);
            continue;
        }

        let Some(mode) = entry.mode else {
            scoped.kind = ScopedRepoFileKind::UnsupportedMode;
            issues.push(FilesIssue {
                path: Some(entry.path.clone()),
                kind: FilesIssueKind::UnknownMode,
                severity: FilesIssueSeverity::Warning,
                message: format!(
                    "{} uses unsupported Git mode {} and cannot be faithfully represented",
                    entry.path, scoped.raw_mode
                ),
            });
            scoped_files.push(scoped);
            continue;
        };

        match mode {
            GitEntryMode::Symlink => {
                scoped.kind = ScopedRepoFileKind::Symlink;
                issues.push(FilesIssue {
                    path: Some(entry.path.clone()),
                    kind: FilesIssueKind::Symlink,
                    severity: FilesIssueSeverity::Warning,
                    message: format!(
                        "{} is a symlink and cannot be copied as an ordinary file",
                        entry.path
                    ),
                });
                scoped_files.push(scoped);
                continue;
            }
            GitEntryMode::Submodule => {
                scoped.kind = ScopedRepoFileKind::Submodule;
                issues.push(FilesIssue {
                    path: Some(entry.path.clone()),
                    kind: FilesIssueKind::Submodule,
                    severity: FilesIssueSeverity::Warning,
                    message: format!(
                        "{} is a submodule entry and cannot be copied as an ordinary file",
                        entry.path
                    ),
                });
                scoped_files.push(scoped);
                continue;
            }
            GitEntryMode::Tree => {
                scoped.kind = ScopedRepoFileKind::UnsupportedMode;
                issues.push(FilesIssue {
                    path: Some(entry.path.clone()),
                    kind: FilesIssueKind::UnknownMode,
                    severity: FilesIssueSeverity::Warning,
                    message: format!(
                        "{} resolved to unsupported Git mode {}",
                        entry.path, scoped.raw_mode
                    ),
                });
                scoped_files.push(scoped);
                continue;
            }
            GitEntryMode::File | GitEntryMode::Executable => {}
        }

        if let Some(size) = entry.size
            && size > MAX_MANAGED_BLOB_BYTES
        {
            scoped.kind = ScopedRepoFileKind::Oversized;
            issues.push(FilesIssue {
                path: Some(entry.path.clone()),
                kind: FilesIssueKind::Oversized,
                severity: FilesIssueSeverity::Warning,
                message: format!(
                    "{} is {} bytes, above the {} byte managed-file limit",
                    entry.path, size, MAX_MANAGED_BLOB_BYTES
                ),
            });
            scoped_files.push(scoped);
            continue;
        }

        let bytes = client.get_blob_bytes(repo, &entry.sha).await?;
        if bytes.len() as u64 > MAX_MANAGED_BLOB_BYTES {
            scoped.kind = ScopedRepoFileKind::Oversized;
            issues.push(FilesIssue {
                path: Some(entry.path.clone()),
                kind: FilesIssueKind::Oversized,
                severity: FilesIssueSeverity::Warning,
                message: format!(
                    "{} expands to {} bytes, above the {} byte managed-file limit",
                    entry.path,
                    bytes.len(),
                    MAX_MANAGED_BLOB_BYTES
                ),
            });
            scoped_files.push(scoped);
            continue;
        }

        if is_lfs_pointer(&bytes) {
            scoped.kind = ScopedRepoFileKind::LfsPointer;
            scoped.bytes = Some(bytes);
            issues.push(FilesIssue {
                path: Some(entry.path.clone()),
                kind: FilesIssueKind::LfsPointer,
                severity: FilesIssueSeverity::Warning,
                message: format!(
                    "{} is a Git LFS pointer; Ward will not attempt to copy the external payload",
                    entry.path
                ),
            });
            scoped_files.push(scoped);
            continue;
        }

        let managed = managed_file_from_bytes(&entry.path, &bytes, mode, &entry.sha);
        scoped.bytes = Some(bytes);
        managed_entries.push(managed);
        scoped_files.push(scoped);
    }

    managed_entries.sort_by(|left, right| left.path.cmp(&right.path));
    scoped_files.sort_by(|left, right| left.path.cmp(&right.path));

    let policy = category
        .map(|category| category.policy.clone())
        .unwrap_or_else(|| {
            if managed_entries.is_empty() {
                CategoryPolicy::observe()
            } else {
                CategoryPolicy::managed()
            }
        });

    Ok(FilesCollection {
        category: FilesCategoryV2 {
            policy,
            include: scope.include,
            exclude: scope.exclude,
            entries: managed_entries,
        },
        scoped_files,
        issues,
        coverage,
        truncated: tree.truncated,
    })
}

pub fn plan_files_category(
    desired: &FilesCategoryV2,
    actual: &FilesCollection,
) -> Result<FilesPlan> {
    let scope = FileScope::from_category(Some(desired));
    let mut issues = actual.issues.clone();
    issues.extend(validate_desired_category(desired, &scope));

    if desired.policy.disposition != ManagementDisposition::Managed {
        return Ok(FilesPlan {
            upserts: Vec::new(),
            deletions: Vec::new(),
            unchanged: Vec::new(),
            atomic_entries: Vec::new(),
            issues,
        });
    }

    let actual_by_path: BTreeMap<&str, &ScopedRepoFile> = actual
        .scoped_files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();

    let mut desired_paths = BTreeSet::new();
    let mut upserts = Vec::new();
    let mut unchanged = Vec::new();

    for desired_file in &desired.entries {
        desired_paths.insert(desired_file.path.clone());

        let desired_mode = parse_managed_mode(&desired_file.mode, &desired_file.path, &mut issues);
        let desired_bytes = decode_managed_file(desired_file, &mut issues);

        let Some(desired_mode) = desired_mode else {
            continue;
        };
        let Some(desired_bytes) = desired_bytes else {
            continue;
        };

        let Some(actual_file) = actual_by_path.get(desired_file.path.as_str()) else {
            upserts.push(desired_file.clone());
            continue;
        };

        if actual_file.kind != ScopedRepoFileKind::Managed {
            upserts.push(desired_file.clone());
            continue;
        }

        if actual_file.mode == Some(desired_mode)
            && actual_file.bytes.as_deref() == Some(desired_bytes.as_slice())
        {
            unchanged.push(desired_file.path.clone());
        } else {
            upserts.push(desired_file.clone());
        }
    }

    let mut deletions = Vec::new();
    if desired.policy.prune {
        if actual.truncated {
            issues.push(FilesIssue {
                path: None,
                kind: FilesIssueKind::TruncatedTree,
                severity: FilesIssueSeverity::Blocker,
                message: "Refusing to prune files from a truncated Git tree listing".to_owned(),
            });
        }

        for file in &actual.scoped_files {
            if desired_paths.contains(&file.path) {
                if file.kind != ScopedRepoFileKind::Managed {
                    issues.push(prune_blocker_for_unsupported(file));
                }
                continue;
            }

            if file.kind != ScopedRepoFileKind::Managed {
                issues.push(prune_blocker_for_unsupported(file));
                continue;
            }

            let Some(mode) = file.mode else {
                issues.push(prune_blocker_for_unsupported(file));
                continue;
            };

            deletions.push(DeleteTreeEntry {
                path: file.path.clone(),
                mode,
                object_type: file.object_type,
            });
        }
    }

    upserts.sort_by(|left, right| left.path.cmp(&right.path));
    deletions.sort_by(|left, right| left.path.cmp(&right.path));
    unchanged.sort();

    let mut atomic_entries = Vec::with_capacity(upserts.len() + deletions.len());
    for file in &upserts {
        if let Some(entry) = managed_file_to_atomic_entry(file, &mut issues) {
            atomic_entries.push(entry);
        }
    }
    atomic_entries.extend(deletions.iter().cloned().map(AtomicCommitEntry::Delete));

    Ok(FilesPlan {
        upserts,
        deletions,
        unchanged,
        atomic_entries,
        issues,
    })
}

pub async fn apply_files_plan(
    client: &Client,
    repo: &str,
    branch: &str,
    message: &str,
    plan: &FilesPlan,
) -> Result<FilesApplyResult> {
    if let Some(blocker) = plan
        .issues
        .iter()
        .find(|issue| issue.severity == FilesIssueSeverity::Blocker)
    {
        anyhow::bail!("Files plan is blocked: {}", blocker.message);
    }

    if plan.atomic_entries.is_empty() {
        return Ok(FilesApplyResult {
            commit_sha: None,
            entry_count: 0,
        });
    }

    let commit_sha = client
        .create_atomic_commit(repo, branch, message, &plan.atomic_entries)
        .await?;

    Ok(FilesApplyResult {
        commit_sha: Some(commit_sha),
        entry_count: plan.atomic_entries.len(),
    })
}

pub async fn verify_files_category(
    client: &Client,
    repo: &str,
    branch: Option<&str>,
    desired: &FilesCategoryV2,
) -> Result<FilesVerifyResult> {
    let actual = collect_files_category(client, repo, branch, Some(desired)).await?;
    let plan = plan_files_category(desired, &actual)?;
    let matches = plan.atomic_entries.is_empty()
        && plan
            .issues
            .iter()
            .all(|issue| issue.severity != FilesIssueSeverity::Blocker);

    Ok(FilesVerifyResult { matches, plan })
}

fn validate_desired_category(desired: &FilesCategoryV2, scope: &FileScope) -> Vec<FilesIssue> {
    let mut issues = Vec::new();
    let mut seen_paths = BTreeSet::new();

    for file in &desired.entries {
        if !seen_paths.insert(file.path.clone()) {
            issues.push(FilesIssue {
                path: Some(file.path.clone()),
                kind: FilesIssueKind::DuplicateDesiredPath,
                severity: FilesIssueSeverity::Blocker,
                message: format!("{} appears more than once in files.entries", file.path),
            });
        }

        if let Err(error) = validate_relative_git_path(&file.path) {
            issues.push(FilesIssue {
                path: Some(file.path.clone()),
                kind: FilesIssueKind::UnsafePath,
                severity: FilesIssueSeverity::Blocker,
                message: error.to_string(),
            });
        }

        if scope.is_excluded(&file.path) {
            issues.push(FilesIssue {
                path: Some(file.path.clone()),
                kind: FilesIssueKind::ExcludedDesiredPath,
                severity: FilesIssueSeverity::Blocker,
                message: format!(
                    "{} is explicitly managed but also matches an exclude pattern",
                    file.path
                ),
            });
        }

        parse_managed_mode(&file.mode, &file.path, &mut issues);
        decode_managed_file(file, &mut issues);
    }

    issues
}

fn parse_managed_mode(
    mode: &str,
    path: &str,
    issues: &mut Vec<FilesIssue>,
) -> Option<GitEntryMode> {
    match mode.parse::<GitEntryMode>() {
        Ok(parsed) if parsed.supports_blob_write() => Some(parsed),
        Ok(parsed) => {
            issues.push(FilesIssue {
                path: Some(path.to_owned()),
                kind: FilesIssueKind::UnsupportedMode,
                severity: FilesIssueSeverity::Blocker,
                message: format!(
                    "{} uses unsupported managed-file mode {}; only 100644 and 100755 are allowed",
                    path, parsed
                ),
            });
            None
        }
        Err(error) => {
            issues.push(FilesIssue {
                path: Some(path.to_owned()),
                kind: FilesIssueKind::UnsupportedMode,
                severity: FilesIssueSeverity::Blocker,
                message: error.to_string(),
            });
            None
        }
    }
}

fn decode_managed_file(file: &ManagedFileV2, issues: &mut Vec<FilesIssue>) -> Option<Vec<u8>> {
    match file.encoding {
        FileEncoding::Utf8 => Some(file.content.as_bytes().to_vec()),
        FileEncoding::Base64 => {
            let cleaned: String = file
                .content
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &cleaned) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    issues.push(FilesIssue {
                        path: Some(file.path.clone()),
                        kind: FilesIssueKind::InvalidBase64,
                        severity: FilesIssueSeverity::Blocker,
                        message: format!("{} has invalid base64 content: {error}", file.path),
                    });
                    None
                }
            }
        }
    }
}

fn managed_file_to_atomic_entry(
    file: &ManagedFileV2,
    issues: &mut Vec<FilesIssue>,
) -> Option<AtomicCommitEntry> {
    let mode = parse_managed_mode(&file.mode, &file.path, issues)?;
    let content = match file.encoding {
        FileEncoding::Utf8 => CommitContent::Utf8(file.content.clone()),
        FileEncoding::Base64 => {
            decode_managed_file(file, issues)?;
            CommitContent::Base64(file.content.clone())
        }
    };

    Some(AtomicCommitEntry::Upsert(AtomicCommitFile {
        path: file.path.clone(),
        mode,
        content,
    }))
}

fn managed_file_from_bytes(
    path: &str,
    bytes: &[u8],
    mode: GitEntryMode,
    sha: &str,
) -> ManagedFileV2 {
    match safe_utf8_text(bytes) {
        Some(text) => ManagedFileV2 {
            path: path.to_owned(),
            content: text,
            encoding: FileEncoding::Utf8,
            mode: mode.as_str().to_owned(),
            source_sha: Some(sha.to_owned()),
        },
        None => ManagedFileV2 {
            path: path.to_owned(),
            content: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
            encoding: FileEncoding::Base64,
            mode: mode.as_str().to_owned(),
            source_sha: Some(sha.to_owned()),
        },
    }
}

fn safe_utf8_text(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8(bytes.to_vec()).ok()?;
    if text.chars().all(is_safe_text_char) {
        Some(text)
    } else {
        None
    }
}

fn is_safe_text_char(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\t') || (!ch.is_control() && ch != '\u{7f}')
}

fn is_lfs_pointer(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let normalized = text.replace("\r\n", "\n");
    let mut lines = normalized.lines();

    matches!(
        lines.next(),
        Some("version https://git-lfs.github.com/spec/v1")
    ) && normalized.contains("\noid sha256:")
        && normalized.contains("\nsize ")
}

fn coverage_entry(
    repo: &str,
    branch: Option<&str>,
    outcome: CoverageOutcome,
    reason: Option<String>,
    required_permission: Option<&str>,
) -> CoverageEntry {
    CoverageEntry {
        category: ManifestCategoryName::Files,
        endpoint: format!(
            "GET /repos/{{org}}/{repo}/git/trees/*?recursive=1{}",
            branch
                .map(|branch| format!(" (ref {branch})"))
                .unwrap_or_default()
        ),
        outcome,
        reason,
        required_permission: required_permission.map(str::to_owned),
    }
}

fn prune_blocker_for_unsupported(file: &ScopedRepoFile) -> FilesIssue {
    let (kind, subject) = match file.kind {
        ScopedRepoFileKind::Managed => unreachable!("managed file cannot be a prune blocker"),
        ScopedRepoFileKind::UnsupportedMode => (
            FilesIssueKind::UnknownMode,
            format!("unsupported Git mode {}", file.raw_mode),
        ),
        ScopedRepoFileKind::Symlink => (FilesIssueKind::Symlink, "symlink".to_owned()),
        ScopedRepoFileKind::Submodule => (FilesIssueKind::Submodule, "submodule".to_owned()),
        ScopedRepoFileKind::LfsPointer => {
            (FilesIssueKind::LfsPointer, "Git LFS pointer".to_owned())
        }
        ScopedRepoFileKind::Oversized => (FilesIssueKind::Oversized, "oversized blob".to_owned()),
        ScopedRepoFileKind::UnsafePath => (FilesIssueKind::UnsafePath, "unsafe path".to_owned()),
    };

    FilesIssue {
        path: Some(file.path.clone()),
        kind,
        severity: FilesIssueSeverity::Blocker,
        message: format!(
            "Refusing to prune {} because Ward collected it as {subject} and cannot faithfully round-trip it",
            file.path
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FileScope, FilesIssueKind, FilesIssueSeverity, MAX_MANAGED_BLOB_BYTES, decode_managed_file,
        is_lfs_pointer, parse_managed_mode, plan_files_category, safe_utf8_text,
    };
    use crate::config::manifest::{
        CategoryPolicy, CoverageEntry, FileEncoding, FilesCategoryV2, ManagedFileV2,
        ManagementDisposition,
    };
    use crate::github::contents::{GitEntryMode, GitObjectType};
    use crate::reconcile::files::{FilesCollection, ScopedRepoFile, ScopedRepoFileKind};

    #[test]
    fn scope_exclude_overrides_include() {
        let scope = FileScope::from_category(Some(&FilesCategoryV2 {
            policy: CategoryPolicy::managed(),
            include: vec![".github/**".to_owned()],
            exclude: vec![".github/generated/**".to_owned()],
            entries: Vec::new(),
        }));

        assert!(scope.is_selected(".github/workflows/ci.yml"));
        assert!(!scope.is_selected(".github/generated/ci.yml"));
    }

    #[test]
    fn default_known_config_scope_covers_registry_paths() {
        let scope = FileScope::from_category(None);

        assert!(scope.is_selected(".github/workflows/ci.yml"));
        assert!(scope.is_selected(".devcontainer/devcontainer.json"));
        assert!(scope.is_selected("devcontainer.json"));
        assert!(scope.is_selected("CODEOWNERS"));
        assert!(scope.is_selected("docs/CODEOWNERS"));
        assert!(scope.is_selected("SECURITY.md"));
        assert!(scope.is_selected("CONTRIBUTING.md"));
        assert!(scope.is_selected("SUPPORT.md"));
        assert!(scope.is_selected("CODE_OF_CONDUCT.md"));
        assert!(scope.is_selected("GOVERNANCE.md"));
        assert!(scope.is_selected("CITATION.cff"));
        assert!(scope.is_selected(".editorconfig"));
        assert!(scope.is_selected(".gitattributes"));
        assert!(scope.is_selected(".gitignore"));
        assert!(scope.is_selected("sonar-project.properties"));
        assert!(scope.is_selected(".shellcheckrc"));
        assert!(scope.is_selected("renovate.json"));
        assert!(scope.is_selected("renovate.json5"));
        assert!(scope.is_selected(".renovaterc"));
        assert!(scope.is_selected(".renovaterc.json"));
        assert!(scope.is_selected(".commitlintrc"));
        assert!(scope.is_selected(".commitlintrc.yaml"));
        assert!(scope.is_selected("commitlint.config.cjs"));
        assert!(scope.is_selected(".markdownlint.yaml"));
        assert!(scope.is_selected(".yamllint.yml"));
        assert!(scope.is_selected(".pre-commit-config.yaml"));
        assert!(scope.is_selected("lefthook.yml"));
        assert!(scope.is_selected(".releaserc.json"));
        assert!(scope.is_selected("release.config.js"));
        assert!(scope.is_selected("release-please-config.json"));
        assert!(!scope.is_selected("src/app.config.ts"));
        assert!(!scope.is_selected("config/application.yaml"));
        assert!(!scope.is_selected("src/main.rs"));
    }

    #[test]
    fn detects_git_lfs_pointer() {
        let pointer = br#"version https://git-lfs.github.com/spec/v1
oid sha256:0123456789abcdef
size 42
"#;
        assert!(is_lfs_pointer(pointer));
    }

    #[test]
    fn safe_utf8_text_rejects_nul_and_control_bytes() {
        assert_eq!(safe_utf8_text(b"hello\n"), Some("hello\n".to_owned()));
        assert!(safe_utf8_text(b"hello\0world").is_none());
        assert!(safe_utf8_text("bad\u{0001}".as_bytes()).is_none());
    }

    #[test]
    fn decode_managed_file_reports_invalid_base64() {
        let file = ManagedFileV2 {
            path: ".github/logo.png".to_owned(),
            content: "***".to_owned(),
            encoding: FileEncoding::Base64,
            mode: "100644".to_owned(),
            source_sha: None,
        };
        let mut issues = Vec::new();

        assert!(decode_managed_file(&file, &mut issues).is_none());
        assert_eq!(issues[0].kind, FilesIssueKind::InvalidBase64);
        assert_eq!(issues[0].severity, FilesIssueSeverity::Blocker);
    }

    #[test]
    fn parse_managed_mode_rejects_symlink_mode() {
        let mut issues = Vec::new();

        assert!(parse_managed_mode("120000", "link", &mut issues).is_none());
        assert_eq!(issues[0].kind, FilesIssueKind::UnsupportedMode);
    }

    #[test]
    fn prune_blocks_on_truncated_actual_tree() {
        let desired = FilesCategoryV2 {
            policy: CategoryPolicy {
                disposition: ManagementDisposition::Managed,
                prune: true,
                sensitive: false,
            },
            include: vec![".github/**".to_owned()],
            exclude: Vec::new(),
            entries: vec![ManagedFileV2 {
                path: ".github/workflows/ci.yml".to_owned(),
                content: "name: CI\n".to_owned(),
                encoding: FileEncoding::Utf8,
                mode: "100644".to_owned(),
                source_sha: None,
            }],
        };
        let actual = FilesCollection {
            category: desired.clone(),
            scoped_files: vec![ScopedRepoFile {
                path: ".github/workflows/ci.yml".to_owned(),
                mode: Some(GitEntryMode::File),
                raw_mode: "100644".to_owned(),
                object_type: GitObjectType::Blob,
                sha: "abc".to_owned(),
                size: Some(8),
                kind: ScopedRepoFileKind::Managed,
                bytes: Some(b"name: CI\n".to_vec()),
            }],
            issues: Vec::new(),
            coverage: Vec::<CoverageEntry>::new(),
            truncated: true,
        };

        let plan = plan_files_category(&desired, &actual).unwrap();
        assert!(
            plan.issues
                .iter()
                .any(|issue| issue.kind == FilesIssueKind::TruncatedTree
                    && issue.severity == FilesIssueSeverity::Blocker)
        );
    }

    #[test]
    fn oversized_limit_stays_small_for_config_files() {
        assert_eq!(MAX_MANAGED_BLOB_BYTES, 1024 * 1024);
    }
}
