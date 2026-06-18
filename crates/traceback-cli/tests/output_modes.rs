use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn backup_reports_progress_by_default_and_can_disable_it() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "hello").expect("file should be written");
    init(&repository);

    let with_progress = backup(&repository, &source, &[]);
    assert!(with_progress.contains("Progress:"));

    fs::write(source.join("note.txt"), "hello again").expect("file should change");
    let without_progress = backup(&repository, &source, &["--no-progress"]);
    assert!(!without_progress.contains("Progress:"));
    assert!(without_progress.contains("Backup completed."));
}

#[test]
fn quiet_suppresses_successful_backup_output() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "hello").expect("file should be written");
    init(&repository);

    let output = traceback()
        .arg("--quiet")
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn verbose_adds_repository_and_source_details() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "hello").expect("file should be written");
    init(&repository);

    let output = traceback()
        .arg("--verbose")
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Repository:"));
    assert!(stdout.contains("Sources:"));
}

fn init(repository: &std::path::Path) {
    assert!(
        traceback()
            .arg("init")
            .arg(repository)
            .status()
            .expect("init should execute")
            .success()
    );
}

fn backup(repository: &std::path::Path, source: &std::path::Path, flags: &[&str]) -> String {
    let output = traceback()
        .args(flags)
        .arg("backup")
        .arg(source)
        .arg("--repo")
        .arg(repository)
        .output()
        .expect("backup should execute");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).into_owned()
}
