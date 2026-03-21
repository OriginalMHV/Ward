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
            repo: repo.to_owned(),
            action: action.to_owned(),
            status: status.to_owned(),
            before: serde_json::Value::Bool(before),
            after: serde_json::Value::Bool(after),
        };

        let line = serde_json::to_string(&entry)?;

        if let Ok(mut guard) = self.file.lock()
            && let Some(ref mut f) = *guard
        {
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
