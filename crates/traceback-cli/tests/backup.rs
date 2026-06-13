use std::{fs, process::Command};

use serde_json::Value;
use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn backup_creates_snapshot_manifest_and_chunks() {
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
            .unwrap()
            .success()
    );

    let output = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let snapshot_id = snapshot_id_from_output(&stdout);
    let manifest_path = repository
        .join("snapshots")
        .join(format!("{snapshot_id}.json"));
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(manifest_path).expect("manifest should be readable"),
    )
    .expect("manifest should be valid JSON");

    assert_eq!(manifest["state"], "complete");
    assert_eq!(manifest["summary"]["file_count"], 1);
    assert_eq!(manifest["summary"]["logical_bytes"], 5);
    assert!(
        manifest["files"]
            .as_array()
            .expect("files should be an array")
            .iter()
            .any(|file| file["path"] == "source/note.txt" && file["type"] == "file")
    );
    assert!(
        fs::read_dir(repository.join("chunks"))
            .expect("chunks directory should exist")
            .any(|entry| entry.expect("shard should be readable").path().is_dir())
    );
}

#[test]
fn repeated_backup_reuses_existing_chunks() {
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
            .unwrap()
            .success()
    );
    assert!(
        traceback()
            .arg("backup")
            .arg(&source)
            .arg("--repo")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );

    let second = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("second backup should execute");

    assert!(second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stdout).contains("New data stored:      0 B"),
        "second backup should report no newly stored data"
    );
}

#[test]
fn backup_respects_tracebackignore() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join(".tracebackignore"), "*.tmp\n").expect("ignore file should be written");
    fs::write(source.join("skip.tmp"), "ignored").expect("ignored file should be written");
    fs::write(source.join("keep.txt"), "kept").expect("kept file should be written");

    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );

    let output = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Ignored paths:        1"));
    let snapshot_id = snapshot_id_from_output(&stdout);
    let manifest_path = repository
        .join("snapshots")
        .join(format!("{snapshot_id}.json"));
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(manifest_path).expect("manifest should be readable"),
    )
    .expect("manifest should be valid JSON");

    let files = manifest["files"]
        .as_array()
        .expect("files should be an array");
    assert!(files.iter().any(|file| file["path"] == "source/keep.txt"));
    assert!(!files.iter().any(|file| file["path"] == "source/skip.tmp"));
}

#[test]
fn backup_rejects_locked_repository() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source directory should be created");
    fs::write(source.join("note.txt"), "hello").expect("source file should be written");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );
    fs::write(repository.join("locks").join("writer.lock"), "locked")
        .expect("lock file should be written");

    let backup = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(!backup.status.success());
    assert!(String::from_utf8_lossy(&backup.stderr).contains("repository is locked"));
}

#[test]
fn backup_reports_structured_lock_error() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source directory should be created");
    fs::write(source.join("note.txt"), "hello").expect("source file should be written");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );
    fs::write(repository.join("locks").join("writer.lock"), "locked")
        .expect("lock file should be written");

    let backup = traceback()
        .arg("--json")
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(!backup.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&backup.stderr).expect("stderr should be valid JSON");
    assert_eq!(json["error"]["code"], "repository_locked");
    assert!(
        json["error"]["message"]
            .as_str()
            .expect("message should be a string")
            .contains("repository is locked")
    );
}

#[test]
fn backup_rejects_repository_inside_source() {
    let temporary = tempdir().expect("temporary directory should be created");
    let source = temporary.path().join("source");
    let repository = source.join("repo");
    fs::create_dir_all(&source).expect("source should be created");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );

    let output = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("repository path is inside a source tree")
    );
}

fn snapshot_id_from_output(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Snapshot ID:").map(str::trim))
        .expect("output should include snapshot ID")
        .to_owned()
}
