use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn explain_reports_changes_reuse_and_growth() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("kept.txt"), "shared").expect("shared file should be written");
    init(&repository);
    backup(&source, &repository);
    fs::write(source.join("added.txt"), "new content").expect("new file should be written");
    backup(&source, &repository);

    let output = traceback()
        .arg("explain")
        .arg("latest")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("explain should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added paths:          1"));
    assert!(stdout.contains("New chunk content:    11 B"));
    assert!(stdout.contains("Reused chunk content: 6 B"));
    assert!(stdout.contains("source/added.txt: 11 B new chunk content"));
}

#[test]
fn explain_supports_json_output() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "hello").expect("file should be written");
    init(&repository);
    let snapshot = backup(&source, &repository);

    let output = traceback()
        .arg("--json")
        .arg("explain")
        .arg(&snapshot)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("explain should execute");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output should be valid JSON");
    assert_eq!(json["snapshot_id"], snapshot);
    assert_eq!(json["previous_snapshot_id"], serde_json::Value::Null);
    assert_eq!(json["added"], 1);
    assert_eq!(json["new_chunk_bytes"], 5);
    assert_eq!(json["reused_chunk_bytes"], 0);
    assert_eq!(json["growth_contributors"][0]["path"], "source/note.txt");
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

fn backup(source: &std::path::Path, repository: &std::path::Path) -> String {
    let output = traceback()
        .arg("backup")
        .arg(source)
        .arg("--repo")
        .arg(repository)
        .output()
        .expect("backup should execute");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("Snapshot ID:").map(str::trim))
        .expect("output should contain snapshot ID")
        .to_owned()
}
