use ratatui::widgets::ListState;

use crate::cache::DiskCache;
use crate::config::SecurityCheck;
use crate::github::dependency_graph::{DependencyGraphAudit, DependencyGraphStatus};
use crate::github::repos::Repository;
use crate::github::security::SecurityState;

pub(super) const SPINNER_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

pub(super) enum Tab {
    Repos,
    Security,
    Actions,
    Help,
}

pub(super) enum BgMessage {
    ReposLoaded(Vec<RepoEntry>),
    SecurityLoaded(usize, SecurityState, DependencyGraphAudit),
    SecurityApplied(String, std::result::Result<(), String>),
    ProtectionApplied(String, std::result::Result<(), String>),
    TemplateDeployed(String, String, std::result::Result<String, String>),
    SettingsApplied(String, std::result::Result<String, String>),
    CustomCheckLoaded(usize, usize, bool),
    Error(String),
}

pub(super) enum PendingAction {
    ApplySecurity(String),
    ApplyProtection(String),
    SelectTemplate(String),
    DeployTemplate(String, String),
    ApplySettings(String),
    BulkApplySecurity,
}

pub(super) struct App {
    pub(super) tab: Tab,
    pub(super) repos: Vec<RepoEntry>,
    pub(super) list_state: ListState,
    pub(super) filter: String,
    pub(super) is_filtering: bool,
    pub(super) should_quit: bool,
    pub(super) status_msg: String,
    pub(super) systems: Vec<(String, String)>,
    pub(super) selected_system: usize,
    pub(super) loading: bool,
    pub(super) spinner_frame: usize,
    pub(super) pending_confirm: Option<PendingAction>,
    pub(super) cache: std::collections::HashMap<String, Vec<RepoEntry>>,
    pub(super) bulk_progress: Option<(usize, usize)>,
    pub(super) custom_checks: Vec<SecurityCheck>,
    pub(super) disk_cache: Option<DiskCache>,
}

#[derive(Clone)]
pub(super) struct RepoEntry {
    pub(super) repo: Repository,
    pub(super) security: Option<SecurityState>,
    pub(super) dependency_graph: Option<DependencyGraphAudit>,
    pub(super) custom_checks: Vec<Option<bool>>,
}

pub(super) fn has_loaded_repo_data(entry: &RepoEntry) -> bool {
    entry.security.is_some() && entry.dependency_graph.is_some()
}

pub(super) fn dependency_graph_is_ok(status: &DependencyGraphStatus) -> bool {
    matches!(
        status,
        DependencyGraphStatus::Available | DependencyGraphStatus::Empty
    )
}

pub(super) fn repo_is_healthy(
    security: &SecurityState,
    dependency_graph: &DependencyGraphAudit,
) -> bool {
    security.dependabot_alerts
        && security.secret_scanning
        && security.secret_scanning_ai_detection
        && security.push_protection
        && dependency_graph_is_ok(&dependency_graph.status)
}

impl App {
    pub(super) fn new(systems: Vec<(String, String)>, custom_checks: Vec<SecurityCheck>) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            tab: Tab::Repos,
            repos: Vec::new(),
            list_state: state,
            filter: String::new(),
            is_filtering: false,
            should_quit: false,
            status_msg: "Press Enter to load repos".to_owned(),
            systems,
            selected_system: 0,
            loading: false,
            spinner_frame: 0,
            pending_confirm: None,
            cache: std::collections::HashMap::new(),
            bulk_progress: None,
            custom_checks,
            disk_cache: DiskCache::new(),
        }
    }

    pub(super) fn filtered_repos(&self) -> Vec<&RepoEntry> {
        if self.filter.is_empty() {
            return self.repos.iter().collect();
        }

        let terms: Vec<&str> = self.filter.split_whitespace().collect();
        let includes: Vec<&str> = terms
            .iter()
            .filter(|t| !t.starts_with('!'))
            .copied()
            .collect();
        let excludes: Vec<String> = terms
            .iter()
            .filter(|t| t.starts_with('!'))
            .map(|t| t[1..].to_lowercase())
            .collect();

        self.repos
            .iter()
            .filter(|r| {
                let name = r.repo.name.to_lowercase();
                let include_ok = includes.is_empty()
                    || includes
                        .iter()
                        .any(|inc| name.contains(&inc.to_lowercase()));
                let exclude_ok = excludes.iter().all(|exc| !name.contains(exc.as_str()));
                include_ok && exclude_ok
            })
            .collect()
    }

    pub(super) fn selected_repo(&self) -> Option<&RepoEntry> {
        let filtered = self.filtered_repos();
        self.list_state
            .selected()
            .and_then(|i| filtered.get(i).copied())
    }

    pub(super) fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    pub(super) fn is_loading_security(&self) -> bool {
        !self.repos.is_empty()
            && self
                .repos
                .iter()
                .any(|r| !has_loaded_repo_data(r) || r.custom_checks.iter().any(Option::is_none))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn security_state() -> SecurityState {
        SecurityState {
            dependabot_alerts: true,
            dependabot_security_updates: true,
            secret_scanning: true,
            secret_scanning_ai_detection: true,
            push_protection: true,
        }
    }

    fn dependency_graph(status: DependencyGraphStatus) -> DependencyGraphAudit {
        DependencyGraphAudit {
            status,
            reason: "test".to_owned(),
            sbom_generated_at: None,
            package_count: None,
            dependency_count: None,
        }
    }

    #[test]
    fn repo_health_requires_sbom_health() {
        let security = security_state();
        assert!(repo_is_healthy(
            &security,
            &dependency_graph(DependencyGraphStatus::Available)
        ));
        assert!(repo_is_healthy(
            &security,
            &dependency_graph(DependencyGraphStatus::Empty)
        ));
        assert!(!repo_is_healthy(
            &security,
            &dependency_graph(DependencyGraphStatus::Unavailable)
        ));
    }

    #[test]
    fn repo_health_requires_ai_detection() {
        let mut security = security_state();
        security.secret_scanning_ai_detection = false;

        assert!(!repo_is_healthy(
            &security,
            &dependency_graph(DependencyGraphStatus::Available)
        ));
    }
}
