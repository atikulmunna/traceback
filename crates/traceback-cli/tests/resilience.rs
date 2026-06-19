use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn maintenance_commands_reject_active_writer_lock() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");

    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .expect("init should execute")
            .success()
    );

    let _lock =
        traceback_repo::acquire_writer_lock(&repository).expect("writer lock should be acquired");

    let gc = traceback()
        .arg("gc")
        .arg("--repo")
        .arg(&repository)
        .arg("--yes")
        .output()
        .expect("gc should execute");
    assert!(!gc.status.success());
    assert!(String::from_utf8_lossy(&gc.stderr).contains("repository is locked"));

    let prune = traceback()
        .arg("prune")
        .arg("--repo")
        .arg(&repository)
        .arg("--keep-latest")
        .arg("1")
        .arg("--yes")
        .output()
        .expect("prune should execute");
    assert!(!prune.status.success());
    assert!(String::from_utf8_lossy(&prune.stderr).contains("repository is locked"));
}

#[test]
fn recover_cli_removes_interrupted_publish_artifacts_and_is_idempotent() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");

    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .expect("init should execute")
            .success()
    );

    fs::create_dir(repository.join("staging/abandoned"))
        .expect("staging artifact should be created");
    fs::create_dir_all(repository.join("chunks/aa")).expect("chunk shard should be created");
    fs::write(repository.join("chunks/aa/.tmp-abandoned"), "temporary")
        .expect("temporary chunk should be written");

    let first = traceback()
        .arg("recover")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("recover should execute");
    assert!(first.status.success());
    let first_stdout = String::from_utf8_lossy(&first.stdout);
    assert!(first_stdout.contains("Staging entries removed: 1"));
    assert!(first_stdout.contains("Temporary chunks removed: 1"));
    assert!(!repository.join("staging/abandoned").exists());
    assert!(!repository.join("chunks/aa/.tmp-abandoned").exists());

    let second = traceback()
        .arg("recover")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("second recover should execute");
    assert!(second.status.success());
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(second_stdout.contains("Staging entries removed: 0"));
    assert!(second_stdout.contains("Temporary chunks removed: 0"));
}

#[test]
fn backup_recovers_stale_writer_lock_before_writing() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "hello").expect("source file should be written");

    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .expect("init should execute")
            .success()
    );
    fs::write(
        repository.join("locks/writer.lock"),
        format!(
            r#"{{"pid":{},"created_at":"2026-01-01T00:00:00Z"}}"#,
            exited_child_pid()
        ),
    )
    .expect("stale lock should be written");

    let backup = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(backup.status.success());
    assert!(String::from_utf8_lossy(&backup.stdout).contains("Backup completed."));
    assert!(!repository.join("locks/writer.lock").exists());
}

fn exited_child_pid() -> u32 {
    let mut child = short_lived_command()
        .spawn()
        .expect("short-lived child should spawn");
    let pid = child.id();
    child.wait().expect("short-lived child should exit");
    pid
}

#[cfg(windows)]
fn short_lived_command() -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "exit", "0"]);
    command
}

#[cfg(unix)]
fn short_lived_command() -> Command {
    Command::new("true")
}
