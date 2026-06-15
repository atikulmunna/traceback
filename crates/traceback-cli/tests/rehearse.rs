use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn rehearse_verifies_a_restorable_snapshot() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
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
    let snapshot_id = backup_snapshot_id(&source, &repository);

    let rehearsal = traceback()
        .arg("rehearse")
        .arg(&snapshot_id)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("rehearsal should execute");

    assert!(rehearsal.status.success());
    let stdout = String::from_utf8_lossy(&rehearsal.stdout);
    assert!(stdout.contains("Restore rehearsal completed."));
    assert!(stdout.contains("Files verified:       1"));
    assert!(stdout.contains("Result:               PASS"));
}

#[test]
fn rehearse_fails_when_snapshot_cannot_restore() {
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
    let snapshot_id = backup_snapshot_id(&source, &repository);
    remove_first_chunk_file(&repository);

    let rehearsal = traceback()
        .arg("rehearse")
        .arg(&snapshot_id)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("rehearsal should execute");

    assert!(!rehearsal.status.success());
    assert!(String::from_utf8_lossy(&rehearsal.stderr).contains("chunk verification failed"));
    let records = read_history(&repository);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["operation"], "rehearse");
    assert_eq!(records[0]["snapshot_id"], snapshot_id);
    assert_eq!(records[0]["success"], false);
}

fn backup_snapshot_id(source: &std::path::Path, repository: &std::path::Path) -> String {
    let output = traceback()
        .arg("backup")
        .arg(source)
        .arg("--repo")
        .arg(repository)
        .output()
        .expect("backup should execute");
    assert!(output.status.success());
    snapshot_id_from_output(&String::from_utf8_lossy(&output.stdout))
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

fn read_history(repository: &std::path::Path) -> Vec<serde_json::Value> {
    fs::read_to_string(repository.join("logs/operations-v1.jsonl"))
        .expect("history should be readable")
        .lines()
        .map(|line| serde_json::from_str(line).expect("history line should be valid JSON"))
        .collect()
}
