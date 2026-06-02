use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn init_creates_repository() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");

    let output = traceback()
        .arg("init")
        .arg(&repository)
        .output()
        .expect("traceback should execute");

    assert!(output.status.success());
    assert!(repository.join("config.toml").is_file());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Initialized TraceBack repository"));
}

#[test]
fn init_reports_existing_repository() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");

    let first = traceback()
        .arg("init")
        .arg(&repository)
        .output()
        .expect("first initialization should execute");
    assert!(first.status.success());

    let second = traceback()
        .arg("init")
        .arg(&repository)
        .output()
        .expect("second initialization should execute");

    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("is already initialized"));
}

#[test]
fn init_rejects_incompatible_directory() {
    let temporary = tempdir().expect("temporary directory should be created");
    fs::write(temporary.path().join("existing.txt"), "data").expect("test file should be written");

    let output = traceback()
        .arg("init")
        .arg(temporary.path())
        .output()
        .expect("traceback should execute");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("directory is not empty and is not a TraceBack repository")
    );
}
