use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::github::dependency_graph::DependencyGraphAudit;
use crate::github::repos::Repository;
use crate::github::security::SecurityState;

pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(300);

#[derive(Serialize, Deserialize)]
pub struct CachedSystem {
    pub cached_at: String,
    pub repos: Vec<CachedRepoEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CachedRepoEntry {
    pub repo: Repository,
    #[serde(default)]
    pub security: Option<SecurityState>,
    #[serde(default)]
    pub dependency_graph: Option<DependencyGraphAudit>,
}

pub struct DiskCache {
    cache_dir: PathBuf,
}

impl DiskCache {
    pub fn new() -> Option<Self> {
        let cache_dir = dirs::cache_dir()?.join("ward").join("systems");
        Some(Self { cache_dir })
    }

    pub fn load(&self, system_id: &str, max_age: Duration) -> Option<CachedSystem> {
        let path = self.cache_dir.join(format!("{system_id}.json"));
        let content = std::fs::read_to_string(&path).ok()?;
        let cached: CachedSystem = serde_json::from_str(&content).ok()?;

        let cached_time = chrono::DateTime::parse_from_rfc3339(&cached.cached_at).ok()?;
        let age = chrono::Utc::now().signed_duration_since(cached_time);
        if age.num_seconds() < 0 {
            return Some(cached);
        }
        let age_millis = age.num_milliseconds().max(0) as u64;
        if age_millis >= max_age.as_millis() as u64 {
            return None;
        }

        Some(cached)
    }

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

    pub fn invalidate(&self, system_id: &str) -> Result<()> {
        let path = self.cache_dir.join(format!("{system_id}.json"));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn invalidate_all(&self) -> Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

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
                dependency_graph: None,
            })
            .collect();
        self.save(system_id, &cached)
    }
}

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
    use crate::github::dependency_graph::DependencyGraphStatus;

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
            dependency_graph: Some(DependencyGraphAudit {
                status: DependencyGraphStatus::Available,
                reason: "SBOM export succeeded with 2 dependency package(s)".to_owned(),
                sbom_generated_at: Some("2026-04-19T10:00:00Z".to_owned()),
                package_count: Some(3),
                dependency_count: Some(2),
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
        assert!(loaded.repos[0].dependency_graph.is_some());
    }

    #[test]
    fn stale_cache_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };

        let entries = vec![make_entry("repo-x")];
        dc.save("sys02", &entries).unwrap();

        let result = dc.load("sys02", Duration::from_secs(0));
        assert!(result.is_none());
    }

    #[test]
    fn load_legacy_cache_without_dependency_graph() {
        let tmp = tempfile::tempdir().unwrap();
        let dc = DiskCache {
            cache_dir: tmp.path().to_path_buf(),
        };

        let legacy = serde_json::json!({
            "cached_at": chrono::Utc::now().to_rfc3339(),
            "repos": [{
                "repo": {
                    "name": "repo-legacy",
                    "full_name": "test-org/repo-legacy",
                    "archived": false,
                    "default_branch": "main",
                    "description": null,
                    "language": "Rust",
                    "visibility": "private",
                    "security_and_analysis": null,
                    "topics": []
                },
                "security": {
                    "dependabot_alerts": true,
                    "dependabot_security_updates": false,
                    "secret_scanning": true,
                    "secret_scanning_ai_detection": false,
                    "push_protection": true
                }
            }]
        });

        std::fs::write(
            tmp.path().join("legacy.json"),
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let loaded = dc.load("legacy", DEFAULT_MAX_AGE).unwrap();
        assert_eq!(loaded.repos.len(), 1);
        assert!(loaded.repos[0].security.is_some());
        assert!(loaded.repos[0].dependency_graph.is_none());
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
        dc.invalidate("nope").unwrap();
    }
}
