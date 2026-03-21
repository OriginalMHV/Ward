use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;

#[derive(Serialize)]
struct AuditEntry {
    timestamp: String,
    repo: String,
    action: String,
    status: String,
    before: serde_json::Value,
    after: serde_json::Value,
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
        let entry = AuditEntry {
            timestamp: Utc::now().to_rfc3339(),
            repo: repo.to_string(),
            action: action.to_string(),
            status: status.to_string(),
            before: serde_json::Value::Bool(before),
            after: serde_json::Value::Bool(after),
        };

        let line = serde_json::to_string(&entry)?;

        let mut guard = self.file.lock().map_err(|e| anyhow::anyhow!("Audit log mutex poisoned: {e}"))?;
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

        log.log("test-repo", "enable_thing", "success", false, true).unwrap();

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
    fn audit_entry_is_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();

        let log = AuditLog::new_for_test(path.clone(), file);

        log.log("test-repo", "test_action", "success", false, true).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let entry: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(entry["repo"], "test-repo");
        assert_eq!(entry["action"], "test_action");
        assert_eq!(entry["status"], "success");
        assert_eq!(entry["before"], false);
        assert_eq!(entry["after"], true);
        assert!(entry["timestamp"].as_str().unwrap().len() > 0);
    }
}
