use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn doctor_reports_actionable_missing_evidence() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    init(&repository);

    let output = traceback()
        .arg("doctor")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("doctor should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[CRITICAL] backup_age"));
    assert!(stdout.contains("[WARN] rehearsal_history"));
    assert!(stdout.contains("Action:"));
}

#[test]
fn doctor_supports_structured_findings() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "hello").expect("file should be written");
    init(&repository);
    let snapshot = backup(&source, &repository);
    assert!(
        traceback()
            .arg("check")
            .arg("--repo")
            .arg(&repository)
            .status()
            .expect("check should execute")
            .success()
    );
    assert!(
        traceback()
            .arg("rehearse")
            .arg(&snapshot)
            .arg("--repo")
            .arg(&repository)
            .status()
            .expect("rehearse should execute")
            .success()
    );

    let output = traceback()
        .arg("--json")
        .arg("doctor")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("doctor should execute");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output should be valid JSON");
    assert_eq!(json["latest_snapshot_id"], snapshot);
    assert_eq!(json["integrity_passed"], true);
    assert_eq!(json["latest_check_passed"], true);
    assert_eq!(json["latest_rehearsal_passed"], true);
    assert_eq!(json["health_score"], 100);
    assert_eq!(json["scoring_version"], "reliability-v1");
    assert!(
        json["score_categories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|category| {
                category["code"] == "encryption" && category["status"] == "not_evaluated"
            })
    );
    assert!(
        json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["code"].is_string() && finding["level"].is_string())
    );
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
