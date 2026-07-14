use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuditEntry {
    pub timestamp: String,
    pub repo: String,
    pub action: String,
    pub status: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}

pub struct AuditLog {
    path: PathBuf,
    file: Mutex<Option<fs::File>>,
}

impl AuditLog {
    pub fn new() -> Result<Self> {
        let dir = dirs_path()?;
        fs::create_dir_all(&dir).context("Failed to create ~/.ward/ directory")?;

        let path = dir.join("audit.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .context("Failed to open audit log")?;

        Ok(Self {
            path,
            file: Mutex::new(Some(file)),
        })
    }

    pub fn log(
        &self,
        repo: &str,
        action: &str,
        status: &str,
        before: bool,
        after: bool,
    ) -> Result<()> {
        self.log_values(
            repo,
            action,
            status,
            serde_json::Value::Bool(before),
            serde_json::Value::Bool(after),
        )
    }

    pub fn log_values(
        &self,
        repo: &str,
        action: &str,
        status: &str,
        before: serde_json::Value,
        after: serde_json::Value,
    ) -> Result<()> {
        let entry = AuditEntry {
            timestamp: Utc::now().to_rfc3339(),
            repo: repo.to_string(),
            action: action.to_string(),
            status: status.to_string(),
            before,
            after,
        };

        let line = serde_json::to_string(&entry)?;

        let mut guard = self
            .file
            .lock()
            .map_err(|e| anyhow::anyhow!("Audit log mutex poisoned: {e}"))?;
        if let Some(ref mut f) = *guard {
            writeln!(f, "{line}")?;
        }

        tracing::debug!("audit: {line}");
        Ok(())
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

pub fn default_log_path() -> Result<PathBuf> {
    Ok(dirs_path()?.join("audit.log"))
}

pub fn read_entries(path: &Path) -> Result<Vec<AuditEntry>> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open audit log: {}", path.display()))?;

    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<AuditEntry>(trimmed) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!("Skipping malformed audit entry: {e}");
            }
        }
    }

    Ok(entries)
}

fn dirs_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".ward"))
}

#[cfg(test)]
impl AuditLog {
    fn new_for_test(path: PathBuf, file: fs::File) -> Self {
        Self {
            path,
            file: Mutex::new(Some(file)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_log_creates_file_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();

        let log = AuditLog::new_for_test(path.clone(), file);

        log.log("test-repo", "enable_thing", "success", false, true)
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("test-repo"));
        assert!(content.contains("enable_thing"));
        assert!(content.contains("success"));
    }

    #[test]
    fn audit_log_appends_multiple_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();

        let log = AuditLog::new_for_test(path.clone(), file);

        log.log("repo1", "action1", "success", false, true).unwrap();
        log.log("repo2", "action2", "failure", true, false).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("repo1"));
        assert!(lines[1].contains("repo2"));
    }

    #[test]
    fn audit_log_writes_structured_before_and_after_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();

        let log = AuditLog::new_for_test(path.clone(), file);
        log.log_values(
            "repo",
            "update_repository",
            "success",
            serde_json::json!({ "visibility": "private" }),
            serde_json::json!({ "visibility": "internal" }),
        )
        .unwrap();

        let content = std::fs::read_to_string(path).unwrap();
        let entry: AuditEntry = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(entry.before["visibility"], "private");
        assert_eq!(entry.after["visibility"], "internal");
    }

    #[test]
    fn audit_entry_is_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();

        let log = AuditLog::new_for_test(path.clone(), file);

        log.log("test-repo", "test_action", "success", false, true)
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let entry: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(entry["repo"], "test-repo");
        assert_eq!(entry["action"], "test_action");
        assert_eq!(entry["status"], "success");
        assert_eq!(entry["before"], false);
        assert_eq!(entry["after"], true);
        assert!(!entry["timestamp"].as_str().unwrap().is_empty());
    }

    #[test]
    fn read_entries_parses_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();

        let log = AuditLog::new_for_test(path.clone(), file);
        log.log("repo-a", "set_secret_scanning", "success", false, true)
            .unwrap();
        log.log("repo-b", "enable_dependabot_alerts", "success", false, true)
            .unwrap();
        log.log("repo-c", "set_push_protection", "failure", false, true)
            .unwrap();

        let entries = read_entries(&path).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].repo, "repo-a");
        assert_eq!(entries[1].action, "enable_dependabot_alerts");
        assert_eq!(entries[2].status, "failure");
    }

    #[test]
    fn read_entries_skips_empty_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");

        let entry = r#"{"timestamp":"2024-01-01T00:00:00Z","repo":"r","action":"a","status":"success","before":false,"after":true}"#;
        std::fs::write(&path, format!("\n{entry}\n\n{entry}\n")).unwrap();

        let entries = read_entries(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn read_entries_returns_empty_for_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        std::fs::write(&path, "").unwrap();

        let entries = read_entries(&path).unwrap();
        assert!(entries.is_empty());
    }
}
