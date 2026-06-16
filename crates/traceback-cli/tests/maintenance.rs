use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn gc_dry_run_reports_orphans_without_deleting_them() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("kept.txt"), "kept").expect("file should be written");
    init(&repository);
    backup(&source, &repository);
    let orphan = store_orphan_chunk(&repository, b"orphan");

    let output = traceback()
        .arg("gc")
        .arg("--repo")
        .arg(&repository)
        .arg("--dry-run")
        .output()
        .expect("gc should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Garbage collection dry run completed."));
    assert!(stdout.contains("Orphaned chunks:      1"));
    assert!(orphan.exists());
}

#[test]
fn gc_yes_removes_orphans_after_confirmation() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("kept.txt"), "kept").expect("file should be written");
    init(&repository);
    backup(&source, &repository);
    let orphan = store_orphan_chunk(&repository, b"orphan");

    let output = traceback()
        .arg("--json")
        .arg("gc")
        .arg("--repo")
        .arg(&repository)
        .arg("--yes")
        .output()
        .expect("gc should execute");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output should be valid JSON");
    assert_eq!(json["dry_run"], false);
    assert_eq!(json["orphaned_chunks"].as_array().unwrap().len(), 1);
    assert!(!orphan.exists());
    assert_eq!(live_chunk_files(&repository), 1);
}

#[test]
fn prune_dry_run_plans_old_snapshots_without_deleting_manifests() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "one").expect("file should be written");
    init(&repository);
    let first = backup(&source, &repository);
    fs::write(source.join("note.txt"), "two").expect("file should change");
    let second = backup(&source, &repository);

    let output = traceback()
        .arg("--json")
        .arg("prune")
        .arg("--repo")
        .arg(&repository)
        .arg("--keep-latest")
        .arg("1")
        .arg("--dry-run")
        .output()
        .expect("prune should execute");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output should be valid JSON");
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["pruned_snapshots"][0], first);
    assert_eq!(json["retained_snapshots"][0], second);
    assert!(
        repository
            .join("snapshots")
            .join(format!("{first}.json"))
            .exists()
    );
}

#[test]
fn prune_yes_removes_selected_manifests_but_leaves_chunks_for_gc() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "one").expect("file should be written");
    init(&repository);
    let first = backup(&source, &repository);
    fs::write(source.join("note.txt"), "two").expect("file should change");
    let second = backup(&source, &repository);
    let chunks_before = live_chunk_files(&repository);

    let output = traceback()
        .arg("prune")
        .arg("--repo")
        .arg(&repository)
        .arg("--keep-latest")
        .arg("1")
        .arg("--yes")
        .output()
        .expect("prune should execute");

    assert!(output.status.success());
    assert!(
        !repository
            .join("snapshots")
            .join(format!("{first}.json"))
            .exists()
    );
    assert!(
        repository
            .join("snapshots")
            .join(format!("{second}.json"))
            .exists()
    );
    assert_eq!(live_chunk_files(&repository), chunks_before);
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
        .expect("backup output should include snapshot ID")
        .to_owned()
}

fn store_orphan_chunk(repository: &std::path::Path, content: &[u8]) -> std::path::PathBuf {
    match traceback_repo::store_chunk(repository, content).expect("orphan chunk should store") {
        traceback_repo::StoreChunkOutcome::Stored(metadata)
        | traceback_repo::StoreChunkOutcome::AlreadyExists(metadata) => repository
            .join("chunks")
            .join(&metadata.hash[..2])
            .join(metadata.hash),
    }
}

fn live_chunk_files(repository: &std::path::Path) -> usize {
    fs::read_dir(repository.join("chunks"))
        .expect("chunks should be readable")
        .flatten()
        .filter(|shard| shard.path().is_dir())
        .flat_map(|shard| fs::read_dir(shard.path()).expect("shard should be readable"))
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .count()
}
