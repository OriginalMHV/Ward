//! Human-readable rendering of unified plan and apply reports.

use crate::reconcile::unified::{CoverageCounts, CoverageOutcomeCount, RepoReport, UnifiedReport};

/// Render a concise, human-readable summary of a unified report.
pub fn render_report(report: &UnifiedReport, title: &str) {
    let mut stdout = std::io::stdout().lock();
    // Nothing useful can be done when stdout is closed.
    if render_report_to(&mut stdout, report, title).is_err() {
        tracing::debug!("could not write the report to stdout");
    }
}

/// Render a command error for stderr.
///
/// Plain mode shows the message and each cause on its own indented line.
/// Verbose mode shows the full debug chain.
pub fn render_error(error: &anyhow::Error, verbose: bool) -> String {
    if verbose {
        return format!("Error: {error:?}");
    }
    let mut text = format!("Error: {error}");
    for cause in error.chain().skip(1) {
        let cause = cause.to_string();
        let mut lines = cause.trim_end().lines();
        if let Some(first) = lines.next() {
            text.push_str("\n  Caused by: ");
            text.push_str(first.trim_end());
        }
        for line in lines {
            text.push_str("\n    ");
            text.push_str(line.trim_end());
        }
    }
    text
}

/// How many details are shown under each category before pointing to `--json`.
const SHOWN_DETAILS: usize = 4;
/// Longest single line of error text shown under a category.
const MAX_ERROR_CHARS: usize = 300;

/// Most lines shown for one reason.
const MAX_ERROR_LINES: usize = 8;

/// Write `text` as a block. The first line follows `first`, later lines are
/// indented under it. Blank lines are dropped and long lines are cut.
fn write_block(
    out: &mut impl std::io::Write,
    first: &str,
    indent: &str,
    text: &str,
) -> std::io::Result<()> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    for (index, line) in lines.iter().take(MAX_ERROR_LINES).enumerate() {
        let prefix = if index == 0 { first } else { indent };
        writeln!(out, "{prefix}{}", cut(line))?;
    }
    let hidden = lines.len().saturating_sub(MAX_ERROR_LINES);
    if hidden > 0 {
        writeln!(out, "{indent}... {hidden} more line(s), use --format json")?;
    }
    Ok(())
}

fn cut(line: &str) -> String {
    let flattened = line.split_whitespace().collect::<Vec<_>>().join(" ");
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
                "    {:<18} {:<9} {status}  {}",
                category.category,
                category.disposition,
                count_summary(
                    category.actionable,
                    category.blocked,
                    category.warnings,
                    category.deferred
                ),
            )?;
            if let Some(reason) = unreadable_reason(&category.coverage_outcomes) {
                writeln!(out, "        could not read: {reason}")?;
            }
            if let Some(error) = category
                .error
                .as_deref()
                .filter(|_| matches!(category.status.as_str(), "failed" | "blocked"))
            {
                write_block(out, "        reason: ", "                ", error)?;
            }
            let advised = category
                .details
                .iter()
                .any(|detail| detail.ends_with("then run ward apply again."));
            if category.deferred > 0 && !advised {
                writeln!(out, "        next: {}", deferred_advice(repo))?;
            }
            for detail in category.details.iter().take(SHOWN_DETAILS) {
                write_block(out, "        - ", "          ", detail)?;
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
        "  Summary: {} to change, {} blocked, {} waiting for a pull request, {} warnings across {} repo(s)",
        style(report.actionable).bold(),
        style(report.blocked).bold(),
        style(report.deferred).bold(),
        style(report.warnings).bold(),
        report.repos.len(),
    )?;
    writeln!(out, "  Coverage: {}", coverage_line(&report.coverage))
}

/// Non-zero counts for one category, or a word that says nothing is pending.
fn count_summary(to_change: usize, blocked: usize, warnings: usize, waiting: usize) -> String {
    let parts: Vec<String> = [
        (to_change, "to change"),
        (blocked, "blocked"),
        (warnings, "warning(s)"),
        (waiting, "waiting for a pull request"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label}"))
    .collect();
    if parts.is_empty() {
        "nothing to change".to_owned()
    } else {
        parts.join(", ")
    }
}

/// Why some reads failed, from the per-outcome tally. `None` when all reads worked.
fn unreadable_reason(outcomes: &[CoverageOutcomeCount]) -> Option<String> {
    let parts: Vec<String> = outcomes
        .iter()
        .filter(|entry| entry.count > 0)
        .filter_map(|entry| {
            let reason = match entry.outcome.as_str() {
                "permission_denied" => "the token has no permission",
                "unavailable" => "GitHub did not return the data",
                _ => return None,
            };
            Some(format!("{} ({reason})", entry.count))
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// What to do about changes that wait for the configuration pull request.
fn deferred_advice(repo: &RepoReport) -> String {
    let url = repo
        .categories
        .iter()
        .flat_map(|category| category.details.iter())
        .find_map(|detail| detail.strip_prefix("pull request: "));
    match url {
        Some(url) => format!("merge {url}, then run ward apply again"),
        None => "merge the configuration pull request, then run ward apply again".to_owned(),
    }
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

        let lines: Vec<&str> = text.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.trim_start().starts_with("reason: PUT failed"))
            .expect("a reason line");
        let next = lines[start + 1];
        assert!(next.trim_start().starts_with("with HTTP 422"), "{next}");
        assert!(next.ends_with("..."), "{next}");
        assert!(next.len() < 340, "{}", next.len());
    }

    #[test]
    fn multi_line_reasons_keep_one_entry_per_indented_line() {
        let error = "Could not write repo to GitHub: PATCH /repos/o/r failed with HTTP 422 Unprocessable Entity. Fix the value below in the manifest, then try again.\n  - Repository.name (invalid): name is too long\n  - Label.color (invalid)\nDocumentation: https://docs.github.com/rest/repos/repos";
        let mut details = vec!["first line\nsecond line".to_owned()];
        details.push("plain".to_owned());
        let text = render(&report_with(category(
            "failed",
            Some(error.to_owned()),
            details,
        )));

        let expected = "        reason: Could not write repo to GitHub: PATCH /repos/o/r failed with HTTP 422 Unprocessable Entity. Fix the value below in the manifest, then try again.\n                - Repository.name (invalid): name is too long\n                - Label.color (invalid)\n                Documentation: https://docs.github.com/rest/repos/repos\n        - first line\n          second line\n        - plain\n";
        assert!(text.contains(expected), "{text}");
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

    #[test]
    fn errors_show_the_message_and_each_cause_on_its_own_line() {
        let error = anyhow::anyhow!("TOML parse error at line 1, column 5\n  |\n1 | [org\n")
            .context("Failed to read ward.toml");

        assert_eq!(
            render_error(&error, false),
            "Error: Failed to read ward.toml\n  Caused by: TOML parse error at line 1, column 5\n      |\n    1 | [org"
        );
    }

    #[test]
    fn verbose_errors_use_the_debug_chain() {
        let error = anyhow::anyhow!("inner").context("outer");

        assert_eq!(
            render_error(&error, true),
            "Error: outer\n\nCaused by:\n    inner"
        );
    }

    #[test]
    fn text_report_uses_plain_words_and_hides_zero_counts() {
        let mut planned = category("planned", None, Vec::new());
        planned.blocked = 1;
        let mut report = report_with(planned);
        report.actionable = 6;
        report.blocked = 1;
        let text = render(&report);

        assert!(text.contains("6 to change, 1 blocked"), "{text}");
        assert!(!text.contains("actionable"), "{text}");
        assert!(!text.contains("deferred"), "{text}");
        assert!(
            text.contains("Summary: 6 to change, 1 blocked, 0 waiting for a pull request"),
            "{text}"
        );

        let quiet = category("noop", None, Vec::new());
        let mut quiet = quiet;
        quiet.actionable = 0;
        assert!(render(&report_with(quiet)).contains("nothing to change"));
    }

    #[test]
    fn text_report_says_why_reads_failed() {
        let mut limited = category("planned", None, Vec::new());
        limited.coverage_outcomes = vec![
            CoverageOutcomeCount {
                outcome: "collected".to_owned(),
                count: 4,
            },
            CoverageOutcomeCount {
                outcome: "permission_denied".to_owned(),
                count: 2,
            },
            CoverageOutcomeCount {
                outcome: "unavailable".to_owned(),
                count: 1,
            },
        ];

        let text = render(&report_with(limited));

        assert!(
            text.contains(
                "could not read: 2 (the token has no permission), 1 (GitHub did not return the data)"
            ),
            "{text}"
        );
    }

    #[test]
    fn deferred_changes_say_which_pull_request_to_merge() {
        let mut files = category(
            "success",
            None,
            vec!["pull request: https://github.com/o/r/pull/7".to_owned()],
        );
        files.category = "files".to_owned();
        let mut waiting = category("deferred", None, Vec::new());
        waiting.deferred = 2;
        waiting.actionable = 0;
        let report = UnifiedReport::from_repos(vec![RepoReport {
            repo: "repo-a".to_owned(),
            categories: vec![files, waiting],
            actionable: 0,
            blocked: 0,
            warnings: 0,
            deferred: 2,
        }]);

        let text = render(&report);

        assert!(text.contains("2 waiting for a pull request"), "{text}");
        assert!(
            text.contains("next: merge https://github.com/o/r/pull/7, then run ward apply again"),
            "{text}"
        );
    }

    #[test]
    fn a_deferred_detail_that_names_the_pull_request_is_not_repeated() {
        let mut waiting = category(
            "deferred",
            None,
            vec![
                "2 change(s) wait for the configuration pull request. Merge https://github.com/o/r/pull/7, then run ward apply again."
                    .to_owned(),
            ],
        );
        waiting.deferred = 2;

        let text = render(&report_with(waiting));

        assert_eq!(text.matches("pull/7").count(), 1, "{text}");
        assert!(!text.contains("next:"), "{text}");
    }
}
