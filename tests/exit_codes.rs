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
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ward.toml");
    std::fs::write(&path, "[org]\nname = \"test-org\"\n").unwrap();
    let home = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ward"))
        .args(["--config", path.to_str().unwrap(), "repos", "inspect", "x"])
        .env("HOME", home.path())
        .env("GH_TOKEN", "dummy-token")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("ward audit --repo"));
}
