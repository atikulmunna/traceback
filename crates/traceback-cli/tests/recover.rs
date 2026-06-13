use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn recover_removes_interrupted_write_artifacts() {
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
    fs::create_dir(repository.join("staging").join("abandoned"))
        .expect("staging artifact should be created");
    let shard = repository.join("chunks").join("aa");
    fs::create_dir(&shard).expect("chunk shard should be created");
    fs::write(shard.join(".tmp-abandoned"), "temporary")
        .expect("temporary chunk should be written");

    let before = traceback()
        .arg("check")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("check should execute");
    assert!(!before.status.success());
    let before_stdout = String::from_utf8_lossy(&before.stdout);
    assert!(before_stdout.contains("Staging leftovers:    1"));
    assert!(before_stdout.contains("Temporary chunks:     1"));

    let recovery = traceback()
        .arg("recover")
        .arg("--repo")
        .arg(&repository)
        .arg("--json")
        .output()
        .expect("recovery should execute");

    assert!(recovery.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&recovery.stdout).expect("output should be valid JSON");
    assert_eq!(json["staging_entries_removed"], 1);
    assert_eq!(json["temporary_chunks_removed"], 1);

    let after = traceback()
        .arg("check")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("check should execute");
    assert!(after.status.success());
}
