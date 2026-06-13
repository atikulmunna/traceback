use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn snapshots_reports_empty_repository() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );

    let output = traceback()
        .arg("snapshots")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("snapshots should execute");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("No snapshots found."));
}

#[test]
fn snapshots_lists_published_backups() {
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
    let backup = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");
    assert!(backup.status.success());
    let snapshot_id = snapshot_id_from_output(&String::from_utf8_lossy(&backup.stdout));

    let output = traceback()
        .arg("snapshots")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("snapshots should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ID"));
    assert!(stdout.contains(&snapshot_id));
    assert!(stdout.contains("5 B"));
    assert!(stdout.contains("source"));
}

#[test]
fn snapshots_supports_json_output() {
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
    let backup = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");
    assert!(backup.status.success());
    let snapshot_id = snapshot_id_from_output(&String::from_utf8_lossy(&backup.stdout));

    let output = traceback()
        .arg("--json")
        .arg("snapshots")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("snapshots should execute");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output should be valid JSON");
    let snapshots = json["snapshots"]
        .as_array()
        .expect("snapshots should be an array");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0]["id"], snapshot_id);
    assert_eq!(snapshots[0]["file_count"], 1);
    assert_eq!(snapshots[0]["logical_bytes"], 5);
    assert_eq!(snapshots[0]["sources"][0], "source");
}

#[test]
fn snapshots_rejects_invalid_published_manifest() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        repository.join("snapshots").join("broken.json"),
        "{not json",
    )
    .expect("broken manifest should be written");

    let output = traceback()
        .arg("snapshots")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("snapshots should execute");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("manifest JSON is invalid"));
}

fn snapshot_id_from_output(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Snapshot ID:").map(str::trim))
        .expect("output should include snapshot ID")
        .to_owned()
}
