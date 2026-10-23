// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "mimecraft-html-entities-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("input.json"),
            r#"{"&amp;": {"codepoints": [38], "characters": "&"}}"#,
        )
        .unwrap();
        Self(directory)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mimecraft-html-entities"));
        command.current_dir(&self.0);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["input.json", "output.rs"] {
            let path = self.0.join(name);
            if path.exists() {
                fs::remove_file(path).unwrap();
            }
        }
        fs::remove_dir(&self.0).unwrap();
    }
}

#[test]
fn explicit_paths_generate_and_check_without_overwriting_stale_output() {
    let fixture = Fixture::new();
    let result = fixture
        .command()
        .args(["input.json", "output.rs"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result);
    let generated = fs::read(fixture.0.join("output.rs")).unwrap();
    assert!(String::from_utf8_lossy(&generated).contains("(\"amp;\", \"\\u{26}\")"));
    let result = fixture
        .command()
        .args(["--check", "input.json", "output.rs"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result);

    fs::write(fixture.0.join("output.rs"), "stale").unwrap();
    let result = fixture
        .command()
        .args(["--check", "input.json", "output.rs"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("is stale"));
    assert_eq!(fs::read(fixture.0.join("output.rs")).unwrap(), b"stale");

    let result = fixture
        .command()
        .args(["input.json", "output.rs"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result);
    assert_eq!(fs::read(fixture.0.join("output.rs")).unwrap(), generated);
}

#[test]
fn invalid_input_and_missing_files_fail_without_replacing_output() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("input.json"), "{}").unwrap();
    fs::write(fixture.0.join("output.rs"), "original").unwrap();
    let result = fixture
        .command()
        .args(["input.json", "output.rs"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("entity table must not be empty"));
    assert_eq!(fs::read(fixture.0.join("output.rs")).unwrap(), b"original");

    let result = fixture
        .command()
        .args(["missing.json", "output.rs"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot read missing.json"));
    assert_eq!(fs::read(fixture.0.join("output.rs")).unwrap(), b"original");
}

#[test]
fn default_check_is_independent_of_working_directory() {
    let fixture = Fixture::new();
    let result = fixture.command().arg("--check").output().unwrap();
    assert!(result.status.success(), "{:?}", result);
    assert!(!fixture.0.join("output.rs").exists());
}

#[test]
fn help_and_invalid_arguments_have_distinct_exit_statuses() {
    let fixture = Fixture::new();
    let result = fixture.command().arg("--help").output().unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("Usage:"));
    let result = fixture.command().arg("--unknown").output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Usage:"));
}
