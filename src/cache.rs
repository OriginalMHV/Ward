use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::github::repos::Repository;
use crate::github::security::SecurityState;

/// Default cache expiry: 5 minutes.
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(300);

/// A timestamped snapshot of repos (+ optional security state) for one system.
#[derive(Serialize, Deserialize)]
pub struct CachedSystem {
    pub cached_at: String,
    pub repos: Vec<CachedRepoEntry>,
}

/// Minimal repo data that gets persisted to disk.
#[derive(Serialize, Deserialize, Clone)]
pub struct CachedRepoEntry {
    pub repo: Repository,
    pub security: Option<SecurityState>,
}

/// Persistent on-disk cache stored under the OS cache directory.
///
/// Layout:
/// ```text
/// <cache_dir>/ward/systems/{system_id}.json
/// ```
pub struct DiskCache {
    cache_dir: PathBuf,
}

impl DiskCache {
    /// Create a new `DiskCache`. Returns `None` when the OS cache directory
    /// cannot be determined (e.g. missing `$HOME`).
    pub fn new() -> Option<Self> {
        let cache_dir = dirs::cache_dir()?.join("ward").join("systems");
        Some(Self { cache_dir })
    }

    /// Try to load a cached system that is younger than `max_age`.
    /// Returns `None` on any failure (missing file, parse error, stale cache).
    pub fn load(&self, system_id: &str, max_age: Duration) -> Option<CachedSystem> {
        let path = self.cache_dir.join(format!("{system_id}.json"));
        let content = std::fs::read_to_string(&path).ok()?;
        let cached: CachedSystem = serde_json::from_str(&content).ok()?;

        let cached_time = chrono::DateTime::parse_from_rfc3339(&cached.cached_at).ok()?;
        let age = chrono::Utc::now().signed_duration_since(cached_time);
        if age.num_seconds() < 0 {
            // Clock skew -- treat as fresh.
            return Some(cached);
        }
        let age_millis = age.num_milliseconds().max(0) as u64;
        if age_millis >= max_age.as_millis() as u64 {
            return None;
        }

        Some(cached)
    }

    /// Persist a system's repos to the cache directory.
    pub fn save(&self, system_id: &str, entries: &[CachedRepoEntry]) -> Result<()> {
        std::fs::create_dir_all(&self.cache_dir)?;
        let cached = CachedSystem {
            cached_at: chrono::Utc::now().to_rfc3339(),
            repos: entries.to_vec(),
        };
        let json = serde_json::to_string_pretty(&cached)?;
        let path = self.cache_dir.join(format!("{system_id}.json"));
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// Remove the cache file for a single system.
    pub fn invalidate(&self, system_id: &str) -> Result<()> {
        let path = self.cache_dir.join(format!("{system_id}.json"));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Remove all cached system files.
    pub fn invalidate_all(&self) -> Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

    /// Convenience: save a slice of `(Repository, Option<SecurityState>)` tuples.
    pub fn save_from_tuples(
        &self,
        system_id: &str,
        entries: &[(Repository, Option<SecurityState>)],
    ) -> Result<()> {
        let cached: Vec<CachedRepoEntry> = entries
            .iter()
            .map(|(repo, sec)| CachedRepoEntry {
                repo: repo.clone(),
                security: sec.clone(),
            })
            .collect();
        self.save(system_id, &cached)
    }
}

/// Format a cache age as a human-friendly string like "2m ago" or "45s ago".
pub fn format_age(cached_at: &str) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(cached_at) else {
        return String::new();
    };
    let secs = chrono::Utc::now()
        .signed_duration_since(ts)
        .num_seconds()
        .max(0) as u64;
    if secs < 60 {
        format!("{secs}s ago")
    } else {
        format!("{}m ago", secs / 60)
    }
}

/// Silently attempt a disk-cache save, logging a warning on failure.
pub fn try_save(disk_cache: &Option<DiskCache>, system_id: &str, entries: &[CachedRepoEntry]) {
    if let Some(dc) = disk_cache {
        if let Err(e) = dc.save(system_id, entries) {
            warn!("disk cache save failed: {e}");
        }
    }
}

/// Silently attempt disk-cache invalidation, logging a warning on failure.
pub fn try_invalidate(disk_cache: &Option<DiskCache>, system_id: &str) {
    if let Some(dc) = disk_cache {
        if let Err(e) = dc.invalidate(system_id) {
            warn!("disk cache invalidate failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str) -> CachedRepoEntry {
        CachedRepoEntry {
            repo: Repository {
                name: name.to_owned(),
                full_name: format!("org/{name}"),
                archived: false,
                default_branch: "main".to_owned(),
                description: None,
                visibility: "private".to_owned(),
                language: None,
                security_and_analysis: None,
                topics: vec![],
            },
            security: Some(SecurityState {
                dependabot_alerts: true,
                dependabot_security_updates: false,
                secret_scanning: true,
                secret_scanning_ai_detection: false,
                push_protection: true,
            }),
        }
    }

    #[test]
    fn round_trip_save_load() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };

        let entries = vec![make_entry("repo-a"), make_entry("repo-b")];
        dc.save("sys01", &entries).unwrap();

        let loaded = dc.load("sys01", DEFAULT_MAX_AGE).unwrap();
        assert_eq!(loaded.repos.len(), 2);
        assert_eq!(loaded.repos[0].repo.name, "repo-a");
        assert!(loaded.repos[0].security.is_some());
    }

    #[test]
    fn stale_cache_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };

        let entries = vec![make_entry("repo-x")];
        dc.save("sys02", &entries).unwrap();

        // Ask for max-age of 0 seconds -- always stale.
        let result = dc.load("sys02", Duration::from_secs(0));
        assert!(result.is_none());
    }

    #[test]
    fn invalidate_removes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };

        dc.save("sys03", &[make_entry("r")]).unwrap();
        assert!(tmp.path().join("sys03.json").exists());

        dc.invalidate("sys03").unwrap();
        assert!(!tmp.path().join("sys03.json").exists());
    }

    #[test]
    fn invalidate_all_removes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };

        dc.save("a", &[make_entry("r1")]).unwrap();
        dc.save("b", &[make_entry("r2")]).unwrap();

        dc.invalidate_all().unwrap();
        assert!(!tmp.path().exists());
    }

    #[test]
    fn load_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };

        assert!(dc.load("nonexistent", DEFAULT_MAX_AGE).is_none());
    }

    #[test]
    fn format_age_seconds() {
        let now = chrono::Utc::now();
        let ts = (now - chrono::Duration::seconds(42)).to_rfc3339();
        let result = format_age(&ts);
        assert!(result.contains("s ago"), "got: {result}");
    }

    #[test]
    fn format_age_minutes() {
        let now = chrono::Utc::now();
        let ts = (now - chrono::Duration::seconds(150)).to_rfc3339();
        let result = format_age(&ts);
        assert!(result.contains("m ago"), "got: {result}");
    }

    #[test]
    fn invalidate_nonexistent_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };
        // Should not error when file doesn't exist.
        dc.invalidate("nope").unwrap();
    }
}
