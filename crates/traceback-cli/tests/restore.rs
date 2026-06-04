use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn restore_reconstructs_backed_up_files() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    let target = temporary.path().join("restored");
    fs::create_dir_all(source.join("notes")).expect("source directory should be created");
    fs::write(source.join("notes").join("note.txt"), "hello")
        .expect("source file should be written");
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

    let restore = traceback()
        .arg("restore")
        .arg(&snapshot_id)
        .arg("--repo")
        .arg(&repository)
        .arg("--target")
        .arg(&target)
        .output()
        .expect("restore should execute");

    assert!(restore.status.success());
    assert_eq!(
        fs::read_to_string(target.join("source").join("notes").join("note.txt"))
            .expect("restored file should be readable"),
        "hello"
    );
    assert!(String::from_utf8_lossy(&restore.stdout).contains("Files restored:       1"));
}

#[test]
fn restore_refuses_to_overwrite_existing_files() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    let target = temporary.path().join("restored");
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
    let backup = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");
    assert!(backup.status.success());
    let snapshot_id = snapshot_id_from_output(&String::from_utf8_lossy(&backup.stdout));
    fs::create_dir_all(target.join("source")).expect("target directory should be created");
    fs::write(target.join("source").join("note.txt"), "existing")
        .expect("existing file should be written");

    let restore = traceback()
        .arg("restore")
        .arg(&snapshot_id)
        .arg("--repo")
        .arg(&repository)
        .arg("--target")
        .arg(&target)
        .output()
        .expect("restore should execute");

    assert!(!restore.status.success());
    assert!(String::from_utf8_lossy(&restore.stderr).contains("restore target already exists"));
    assert_eq!(
        fs::read_to_string(target.join("source").join("note.txt"))
            .expect("existing file should remain readable"),
        "existing"
    );
}

#[test]
fn restore_fails_when_chunk_is_missing() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    let target = temporary.path().join("restored");
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
    let backup = traceback()
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");
    assert!(backup.status.success());
    let snapshot_id = snapshot_id_from_output(&String::from_utf8_lossy(&backup.stdout));
    remove_first_chunk_file(&repository);

    let restore = traceback()
        .arg("restore")
        .arg(&snapshot_id)
        .arg("--repo")
        .arg(&repository)
        .arg("--target")
        .arg(&target)
        .output()
        .expect("restore should execute");

    assert!(!restore.status.success());
    assert!(String::from_utf8_lossy(&restore.stderr).contains("chunk verification failed"));
}

#[test]
fn restore_rejects_manifest_path_escape() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let target = temporary.path().join("restored");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        repository.join("snapshots").join("snap_escape.json"),
        r#"{
  "manifest_version": 0,
  "snapshot_id": "snap_escape",
  "state": "complete",
  "created_at": "2026-06-02T00:00:00Z",
  "sources": ["source"],
  "files": [{
    "path": "../evil.txt",
    "type": "file",
    "size": 0,
    "modified_at": null,
    "content_hash": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adcb2f6d6fd3f4f9e4e2eacb",
    "chunks": [],
    "symlink_target": null
  }],
  "summary": {
    "file_count": 1,
    "logical_bytes": 0,
    "newly_stored_bytes": 0
  }
}"#,
    )
    .expect("malicious manifest should be written");

    let restore = traceback()
        .arg("restore")
        .arg("snap_escape")
        .arg("--repo")
        .arg(&repository)
        .arg("--target")
        .arg(&target)
        .output()
        .expect("restore should execute");

    assert!(!restore.status.success());
    assert!(String::from_utf8_lossy(&restore.stderr).contains("manifest path is invalid"));
    assert!(!temporary.path().join("evil.txt").exists());
}

fn snapshot_id_from_output(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Snapshot ID:").map(str::trim))
        .expect("output should include snapshot ID")
        .to_owned()
}

fn remove_first_chunk_file(repository: &std::path::Path) {
    for shard in fs::read_dir(repository.join("chunks")).expect("chunks should be readable") {
        let shard = shard.expect("shard should be readable").path();
        if !shard.is_dir() {
            continue;
        }
        if let Some(chunk) = fs::read_dir(shard)
            .expect("shard should be readable")
            .next()
        {
            let chunk = chunk.expect("chunk should be readable").path();
            fs::remove_file(chunk).expect("chunk should be removed");
            return;
        }
    }
    panic!("expected at least one chunk file");
}
