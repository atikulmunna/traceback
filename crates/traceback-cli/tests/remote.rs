use std::{fs, process::Command};

use serde_json::Value;
use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn remote_push_copies_repository_objects_to_filesystem_remote() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let remote = temporary.path().join("remote");
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
    assert!(
        traceback()
            .arg("backup")
            .arg(&source)
            .arg("--repo")
            .arg(&repository)
            .status()
            .expect("backup should execute")
            .success()
    );

    let output = traceback()
        .arg("remote")
        .arg("push")
        .arg("--repo")
        .arg(&repository)
        .arg("--remote")
        .arg(format!("file://{}", remote.display()))
        .output()
        .expect("remote push should execute");

    assert!(output.status.success());
    assert!(remote.join("config.toml").is_file());
    assert!(
        fs::read_dir(remote.join("chunks"))
            .expect("remote chunks should exist")
            .any(|entry| entry.expect("remote shard should read").path().is_dir())
    );
    assert!(
        fs::read_dir(remote.join("snapshots"))
            .expect("remote snapshots should exist")
            .any(|entry| entry.expect("remote snapshot should read").path().is_file())
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Remote push completed."));
}

#[test]
fn remote_push_is_idempotent_and_reports_json_counts() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let remote = temporary.path().join("remote");
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
    assert!(
        traceback()
            .arg("backup")
            .arg(&source)
            .arg("--repo")
            .arg(&repository)
            .status()
            .expect("backup should execute")
            .success()
    );
    assert!(
        traceback()
            .arg("remote")
            .arg("push")
            .arg("--repo")
            .arg(&repository)
            .arg("--remote")
            .arg(&remote)
            .status()
            .expect("first remote push should execute")
            .success()
    );

    let output = traceback()
        .arg("--json")
        .arg("remote")
        .arg("push")
        .arg("--repo")
        .arg(&repository)
        .arg("--remote")
        .arg(&remote)
        .output()
        .expect("second remote push should execute");

    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("json should parse");
    assert_eq!(report["copied_files"], 0);
    assert!(
        report["skipped_files"]
            .as_u64()
            .expect("skipped files should be numeric")
            >= 3
    );
}
