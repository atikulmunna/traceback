use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn check_passes_for_valid_backup() {
    let (repository, _source, _temporary) = repository_with_backup();

    let output = traceback()
        .arg("check")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("check should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Result:               PASS"));
    assert!(stdout.contains("Manifests checked:    1"));
    assert!(stdout.contains("Chunks verified:      1"));
}

#[test]
fn check_reports_missing_chunk() {
    let (repository, _source, _temporary) = repository_with_backup();
    remove_first_chunk_file(&repository);

    let output = traceback()
        .arg("check")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("check should execute");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Result:               FAIL"));
    assert!(stdout.contains("referenced chunk is missing or corrupt"));
}

#[test]
fn check_reports_corrupted_chunk() {
    let (repository, _source, _temporary) = repository_with_backup();
    corrupt_first_chunk_file(&repository);

    let output = traceback()
        .arg("check")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("check should execute");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("chunk file has invalid magic"));
}

#[test]
fn check_reports_orphans_and_staging_leftovers() {
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
    fs::create_dir(repository.join("chunks").join("aa")).expect("chunk shard should be created");
    fs::write(
        repository.join("chunks").join("aa").join("a".repeat(64)),
        "orphan",
    )
    .expect("orphaned chunk should be written");
    fs::create_dir(repository.join("staging").join("leftover"))
        .expect("staging leftover should be created");

    let output = traceback()
        .arg("check")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("check should execute");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Orphaned chunks:      1"));
    assert!(stdout.contains("Staging leftovers:    1"));
    assert!(stdout.contains("orphaned chunk is not referenced"));
    assert!(stdout.contains("abandoned staging data found"));
}

#[test]
fn check_reports_invalid_manifest() {
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
        .arg("check")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("check should execute");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Result:               FAIL"));
    assert!(stdout.contains("manifest is invalid"));
}

fn repository_with_backup() -> (std::path::PathBuf, std::path::PathBuf, tempfile::TempDir) {
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

    (repository, source, temporary)
}

fn remove_first_chunk_file(repository: &std::path::Path) {
    let path = first_chunk_file(repository);
    fs::remove_file(path).expect("chunk should be removed");
}

fn corrupt_first_chunk_file(repository: &std::path::Path) {
    let path = first_chunk_file(repository);
    let mut contents = fs::read(&path).expect("chunk should be readable");
    contents[0] = b'X';
    fs::write(path, contents).expect("chunk should be corrupted");
}

fn first_chunk_file(repository: &std::path::Path) -> std::path::PathBuf {
    for shard in fs::read_dir(repository.join("chunks")).expect("chunks should be readable") {
        let shard = shard.expect("shard should be readable").path();
        if !shard.is_dir() {
            continue;
        }
        if let Some(chunk) = fs::read_dir(shard)
            .expect("shard should be readable")
            .next()
        {
            return chunk.expect("chunk should be readable").path();
        }
    }
    panic!("expected at least one chunk file");
}
