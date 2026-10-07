use ward::config::manifest::{
    CategoryPolicy, FileEncoding, FilesCategory, ManagedFile, ManagementDisposition,
};
use ward::github::contents::{GitEntryMode, GitObjectType};
use ward::reconcile::files::{
    FilesCollection, FilesIssueKind, FilesIssueSeverity, ScopedRepoFile, ScopedRepoFileKind,
    plan_files_category,
};

fn desired(path: &str, prune: bool) -> FilesCategory {
    FilesCategory {
        policy: CategoryPolicy {
            disposition: ManagementDisposition::Managed,
            prune,
            sensitive: false,
        },
        include: vec![".github/**".to_owned()],
        exclude: Vec::new(),
        entries: vec![ManagedFile {
            path: path.to_owned(),
            content: "name: CI\n".to_owned(),
            encoding: FileEncoding::Utf8,
            mode: "100644".to_owned(),
            source_sha: None,
        }],
    }
}

fn collection(category: &FilesCategory, kind: ScopedRepoFileKind) -> FilesCollection {
    FilesCollection {
        category: category.clone(),
        scoped_files: vec![ScopedRepoFile {
            path: category.entries[0].path.clone(),
            mode: Some(GitEntryMode::File),
            raw_mode: "100644".to_owned(),
            object_type: GitObjectType::Blob,
            sha: "existing".to_owned(),
            size: Some(10),
            kind,
            bytes: None,
        }],
        issues: Vec::new(),
        coverage: Vec::new(),
        truncated: false,
    }
}

#[test]
fn desired_content_at_unsupported_entry_is_blocked_not_upserted() {
    let cases = [
        (ScopedRepoFileKind::Symlink, FilesIssueKind::Symlink),
        (ScopedRepoFileKind::Submodule, FilesIssueKind::Submodule),
        (ScopedRepoFileKind::LfsPointer, FilesIssueKind::LfsPointer),
        (ScopedRepoFileKind::Oversized, FilesIssueKind::Oversized),
        (
            ScopedRepoFileKind::UnsupportedMode,
            FilesIssueKind::UnknownMode,
        ),
    ];

    for prune in [false, true] {
        for (scoped_kind, issue_kind) in cases {
            let category = desired(".github/workflows/ci.yml", prune);
            let plan = plan_files_category(&category, &collection(&category, scoped_kind)).unwrap();

            assert!(plan.upserts.is_empty(), "{scoped_kind:?} prune={prune}");
            assert!(
                plan.atomic_entries.is_empty(),
                "{scoped_kind:?} prune={prune}"
            );
            let blockers: Vec<_> = plan
                .issues
                .iter()
                .filter(|issue| {
                    issue.kind == issue_kind && issue.severity == FilesIssueSeverity::Blocker
                })
                .collect();
            assert_eq!(blockers.len(), 1, "{scoped_kind:?} prune={prune}");
            assert!(blockers[0].message.contains("Refusing to overwrite"));
        }
    }
}
