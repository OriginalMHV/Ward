#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside #[test] functions"
)]

use std::process::Command;

fn ward(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_ward"))
        .args(args)
        .env("HOME", home.path())
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .output()
        .unwrap()
}

#[test]
fn invalid_configuration_exits_with_two() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ward.toml");
    std::fs::write(&path, "this is not valid toml [[[").unwrap();

    let output = ward(&["--config", path.to_str().unwrap(), "plan"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn doctor_with_a_failed_check_exits_with_one() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.toml");

    let output = ward(&["--config", missing.to_str().unwrap(), "doctor"]);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn successful_commands_exit_with_zero() {
    let output = ward(&["completions", "bash"]);

    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn removed_config_subcommands_exit_with_two_and_name_the_replacement() {
    for name in ["set", "add-system", "remove-system"] {
        let output = ward(&["config", name, "org.name", "x"]);

        assert_eq!(output.status.code(), Some(2), "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("ward config edit"), "{stderr}");
    }
}

#[test]
fn removed_repos_inspect_exits_with_two_and_names_the_replacement() {
    // No manifest and no token: the hint must not depend on either.
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ward"))
        .args(["repos", "inspect", "x"])
        .current_dir(dir.path())
        .env("HOME", home.path())
        .env("PATH", dir.path())
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("ward audit --repo"));
}

fn run_with_config(args: &[&str], manifest: &str, token: Option<&str>) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ward.toml");
    std::fs::write(&path, manifest).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ward"));
    command
        .arg("--config")
        .arg(&path)
        .args(args)
        .env("HOME", home.path())
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .stdin(std::process::Stdio::null());
    if let Some(token) = token {
        command.env("GH_TOKEN", token);
    }
    command.output().unwrap()
}

const SECURITY_MANIFEST: &str = "[org]\nname = \"test-org\"\n\n[categories.security]\nsecret_scanning = true\n\n[categories.security.policy]\ndisposition = \"managed\"\n";

#[test]
fn settings_copilot_review_prints_the_snippet_and_exits_with_two_without_a_token() {
    let output = ward(&["settings", "plan", "--ruleset", "copilot-review"]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[[categories.rulesets.repository_rulesets]]"),
        "{stderr}"
    );
    assert!(stderr.contains("copilot_code_review"), "{stderr}");
}

#[test]
fn repos_inspect_hint_needs_no_token_or_manifest() {
    let output = ward(&["repos", "inspect", "x"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("ward audit --repo"));
}

#[test]
fn deprecated_commands_warn_before_they_need_a_token() {
    for (args, warning) in [
        (
            vec!["security", "plan"],
            "warning: 'ward security plan' is deprecated and will be removed in 0.6.0; use 'ward plan --category security'",
        ),
        (
            vec!["teams", "audit"],
            "warning: 'ward teams audit' is deprecated and will be removed in 0.6.0; use 'ward drift --category access'",
        ),
        (
            vec!["drift", "check"],
            "warning: 'ward drift check' is deprecated; use 'ward drift'",
        ),
    ] {
        let output = run_with_config(&args, SECURITY_MANIFEST, None);

        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(warning), "{args:?}: {stderr}");
    }
}

#[test]
fn apply_without_yes_in_a_non_interactive_session_exits_with_two() {
    let output = run_with_config(&["apply"], SECURITY_MANIFEST, Some("dummy-token"));

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to prompt in a non-interactive session; pass --yes"),
        "{stderr}"
    );
}

#[test]
fn target_flags_before_the_subcommand_are_rejected() {
    let output = ward(&["--repo", "x", "plan"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn plan_help_lists_target_and_format_options() {
    let output = ward(&["plan", "--help"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in ["--org", "--system", "--repo", "--format", "--category"] {
        assert!(stdout.contains(flag), "{flag}: {stdout}");
    }
    assert!(!stdout.contains("--json"), "{stdout}");
}
