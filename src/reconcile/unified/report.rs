use serde::Serialize;

// ---------------------------------------------------------------------------
// Serializable report shape (stable JSON contract)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
pub struct CoverageCounts {
    pub total: usize,
    pub collected: usize,
    /// Reads that failed: permission denied or unavailable.
    pub degraded: usize,
    pub not_applicable: usize,
    /// Settings GitHub does not expose, or values it never returns, such as secrets.
    pub unsupported: usize,
}

#[derive(Debug, Serialize)]
pub struct CoverageOutcomeCount {
    pub outcome: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct CategoryReport {
    pub category: String,
    pub disposition: String,
    pub status: String,
    pub actionable: usize,
    pub blocked: usize,
    pub warnings: usize,
    pub deferred: usize,
    pub coverage: CoverageCounts,
    pub coverage_outcomes: Vec<CoverageOutcomeCount>,
    pub details: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(skip_serializing_if = "is_false")]
    pub configuration_pull_request_pending: bool,
}

#[derive(Debug, Serialize)]
pub struct RepoReport {
    pub repo: String,
    pub categories: Vec<CategoryReport>,
    pub actionable: usize,
    pub blocked: usize,
    pub warnings: usize,
    pub deferred: usize,
}

#[derive(Debug, Serialize)]
pub struct UnifiedReport {
    pub repos: Vec<RepoReport>,
    pub actionable: usize,
    pub blocked: usize,
    pub warnings: usize,
    pub deferred: usize,
    pub coverage: CoverageCounts,
}

impl UnifiedReport {
    pub fn from_repos(repos: Vec<RepoReport>) -> Self {
        let mut coverage = CoverageCounts::default();
        let mut actionable = 0;
        let mut blocked = 0;
        let mut warnings = 0;
        let mut deferred = 0;
        for repo in &repos {
            actionable += repo.actionable;
            blocked += repo.blocked;
            warnings += repo.warnings;
            deferred += repo.deferred;
            for category in &repo.categories {
                coverage.total += category.coverage.total;
                coverage.collected += category.coverage.collected;
                coverage.degraded += category.coverage.degraded;
                coverage.not_applicable += category.coverage.not_applicable;
                coverage.unsupported += category.coverage.unsupported;
            }
        }
        Self {
            repos,
            actionable,
            blocked,
            warnings,
            deferred,
            coverage,
        }
    }

    /// How many categories ended as `failed` and as `blocked`, across all repositories.
    pub fn category_problem_counts(&self) -> (usize, usize) {
        let count = |status: &str| {
            self.repos
                .iter()
                .flat_map(|repo| &repo.categories)
                .filter(|category| category.status == status)
                .count()
        };
        (count("failed"), count("blocked"))
    }

    /// Whether any category is blocked or failed (drives a non-zero exit).
    pub fn has_failures(&self) -> bool {
        self.blocked > 0
            || self.repos.iter().any(|repo| {
                repo.categories
                    .iter()
                    .any(|category| category.status == "failed")
            })
    }
}

impl UnifiedReport {
    /// Whether a managed category could not read part of its state because of
    /// a missing permission or an unavailable endpoint. That state is unknown,
    /// not clean. Observe categories are allowed to degrade.
    pub fn has_unknown_managed_state(&self) -> bool {
        self.repos.iter().any(|repo| {
            repo.categories.iter().any(|category| {
                category.disposition == "managed"
                    && category.coverage_outcomes.iter().any(|entry| {
                        matches!(entry.outcome.as_str(), "permission_denied" | "unavailable")
                    })
            })
        })
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}
