use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn diff_reports_changed_paths_between_backups() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("unchanged.txt"), "same").expect("unchanged file should be written");
    fs::write(source.join("modified.txt"), "old").expect("modified file should be written");
    fs::write(source.join("removed.txt"), "removed").expect("removed file should be written");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );
    let old_snapshot = backup_snapshot_id(&source, &repository);

    fs::write(source.join("modified.txt"), "new").expect("modified file should change");
    fs::remove_file(source.join("removed.txt")).expect("removed file should be deleted");
    fs::write(source.join("added.txt"), "added").expect("added file should be written");
    let new_snapshot = backup_snapshot_id(&source, &repository);

    let output = traceback()
        .arg("diff")
        .arg(&old_snapshot)
        .arg(&new_snapshot)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("diff should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added:                1"));
    assert!(stdout.contains("Removed:              1"));
    assert!(stdout.contains("Modified:             1"));
    assert!(stdout.contains("A source/added.txt"));
    assert!(stdout.contains("R source/removed.txt"));
    assert!(stdout.contains("M source/modified.txt"));
}

#[test]
fn diff_reports_no_changes_for_identical_snapshots() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "same").expect("source file should be written");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );
    let snapshot = backup_snapshot_id(&source, &repository);

    let output = traceback()
        .arg("diff")
        .arg(&snapshot)
        .arg(&snapshot)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("diff should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added:                0"));
    assert!(stdout.contains("Removed:              0"));
    assert!(stdout.contains("Modified:             0"));
    assert!(stdout.contains("No path changes found."));
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
