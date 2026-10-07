//! Human-readable rendering of unified plan and apply reports.

use crate::reconcile::unified::{CoverageCounts, UnifiedReport};

/// Render a concise, human-readable summary of a unified report.
pub fn render_report(report: &UnifiedReport, title: &str) {
    let mut stdout = std::io::stdout().lock();
    // Nothing useful can be done when stdout is closed.
    if render_report_to(&mut stdout, report, title).is_err() {
        tracing::debug!("could not write the report to stdout");
    }
}

/// How many details are shown under each category before pointing to `--json`.
const SHOWN_DETAILS: usize = 4;
/// Longest error text shown under a category.
const MAX_ERROR_CHARS: usize = 300;

fn one_line_error(error: &str) -> String {
    let flattened = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.chars().count() <= MAX_ERROR_CHARS {
        return flattened;
    }
    let mut truncated: String = flattened.chars().take(MAX_ERROR_CHARS).collect();
    truncated.push_str("...");
    truncated
}

pub fn render_report_to(
    out: &mut impl std::io::Write,
    report: &UnifiedReport,
    title: &str,
) -> std::io::Result<()> {
    use console::style;

    writeln!(out)?;
    writeln!(out, "  {}", style(title).bold().cyan())?;

    for repo in &report.repos {
        writeln!(out)?;
        writeln!(out, "  {}", style(&repo.repo).bold())?;
        for category in &repo.categories {
            if category.status == "skipped" {
                continue;
            }
            let status = style_status(&category.status);
            writeln!(
                out,
                "    {:<18} {:<9} {status}  actionable={} blocked={} warnings={} deferred={}",
                category.category,
                category.disposition,
                category.actionable,
                category.blocked,
                category.warnings,
                category.deferred,
            )?;
            if let Some(error) = category
                .error
                .as_deref()
                .filter(|_| matches!(category.status.as_str(), "failed" | "blocked"))
            {
                writeln!(out, "        reason: {}", one_line_error(error))?;
            }
            for detail in category.details.iter().take(SHOWN_DETAILS) {
                writeln!(out, "        - {detail}")?;
            }
            let hidden = category.details.len().saturating_sub(SHOWN_DETAILS);
            if hidden > 0 {
                writeln!(out, "        - {hidden} more, use --format json")?;
            }
        }
    }

    writeln!(out)?;
    writeln!(
        out,
        "  Summary: {} actionable, {} blocked, {} deferred, {} warnings across {} repo(s)",
        style(report.actionable).bold(),
        style(report.blocked).bold(),
        style(report.deferred).bold(),
        style(report.warnings).bold(),
        report.repos.len(),
    )?;
    writeln!(out, "  Coverage: {}", coverage_line(&report.coverage))
}

fn coverage_line(counts: &CoverageCounts) -> String {
    let mut parts = vec![format!("{}/{} read", counts.collected, counts.total)];
    if counts.degraded > 0 {
        parts.push(format!("{} could not be read", counts.degraded));
    }
    if counts.unsupported > 0 {
        parts.push(format!("{} not exposed by GitHub", counts.unsupported));
    }
    if counts.not_applicable > 0 {
        parts.push(format!("{} not applicable", counts.not_applicable));
    }
    parts.join(", ")
}

fn style_status(status: &str) -> console::StyledObject<&str> {
    use console::style;
    match status {
        "success" => style(status).green(),
        "noop" | "observed" => style(status).dim(),
        "deferred" => style(status).yellow(),
        "planned" => style(status).cyan(),
        "blocked" | "failed" => style(status).red(),
        _ => style(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::unified::{CategoryReport, RepoReport};

    fn render(report: &UnifiedReport) -> String {
        let mut out = Vec::new();
        render_report_to(&mut out, report, "Test").unwrap();
        String::from_utf8(out).unwrap()
    }

    fn category(status: &str, error: Option<String>, details: Vec<String>) -> CategoryReport {
        CategoryReport {
            category: "branch-protection".to_owned(),
            disposition: "managed".to_owned(),
            status: status.to_owned(),
            actionable: 6,
            blocked: 0,
            warnings: 0,
            deferred: 0,
            coverage: CoverageCounts::default(),
            coverage_outcomes: Vec::new(),
            details,
            error,
            verified: None,
            configuration_pull_request_pending: false,
        }
    }

    fn report_with(category: CategoryReport) -> UnifiedReport {
        UnifiedReport::from_repos(vec![RepoReport {
            repo: "repo-a".to_owned(),
            categories: vec![category],
            actionable: 0,
            blocked: 0,
            warnings: 0,
            deferred: 0,
        }])
    }

    #[test]
    fn text_report_shows_the_reason_under_a_failed_category() {
        let error = format!("PUT failed\nwith HTTP 422: {}", "x".repeat(500));
        let category = category("failed", Some(error), Vec::new());

        let text = render(&report_with(category));

        let reason = text
            .lines()
            .find(|line| line.trim_start().starts_with("reason:"))
            .expect("a reason line");
        assert!(reason.contains("PUT failed with HTTP 422"));
        assert!(reason.ends_with("..."), "{reason}");
        assert!(reason.len() < 340, "{}", reason.len());
    }

    #[test]
    fn text_report_counts_hidden_details() {
        let details = (0..6).map(|n| format!("change {n}")).collect();
        let category = category("planned", None, details);

        let text = render(&report_with(category));

        assert!(text.contains("change 3"));
        assert!(!text.contains("change 4"));
        assert!(text.contains("2 more, use --format json"), "{text}");

        let counts = CoverageCounts {
            total: 5,
            collected: 1,
            degraded: 1,
            not_applicable: 1,
            unsupported: 2,
        };
        assert_eq!(
            coverage_line(&counts),
            "1/5 read, 1 could not be read, 2 not exposed by GitHub, 1 not applicable"
        );
    }
}
