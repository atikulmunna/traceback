use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn blame_size_reports_reconciled_file_and_directory_storage() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir_all(source.join("dir")).expect("source should be created");
    fs::write(source.join("shared.txt"), "shared").expect("shared file should be written");
    init(&repository);
    backup(&source, &repository);
    fs::write(source.join("dir/a.txt"), "inside").expect("first file should be written");
    fs::write(source.join("dir/b.txt"), "inside").expect("second file should be written");
    backup(&source, &repository);

    let output = traceback()
        .arg("--json")
        .arg("blame-size")
        .arg("latest")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("blame-size should execute");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output should be valid JSON");
    assert!(
        json["accounting_method"]
            .as_str()
            .unwrap()
            .contains("assigned once")
    );
    assert!(json["unique_stored_bytes"].as_u64().unwrap() > 0);
    assert!(json["shared_stored_bytes"].as_u64().unwrap() > 0);
    let files = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["file_type"] == "file")
        .collect::<Vec<_>>();
    let attributed: u64 = files
        .iter()
        .map(|entry| {
            entry["unique_stored_bytes"].as_u64().unwrap()
                + entry["shared_stored_bytes"].as_u64().unwrap()
        })
        .sum();
    assert_eq!(
        attributed,
        json["unique_stored_bytes"].as_u64().unwrap()
            + json["shared_stored_bytes"].as_u64().unwrap()
    );
    let directory = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "source/dir")
        .expect("directory should be reported");
    assert!(directory["reclaimable_stored_bytes"].as_u64().unwrap() > 0);
}

#[test]
fn blame_size_prints_accounting_method() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("note.txt"), "hello").expect("file should be written");
    init(&repository);
    backup(&source, &repository);

    let output = traceback()
        .arg("blame-size")
        .arg("latest")
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("blame-size should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Accounting method:"));
    assert!(stdout.contains("source/note.txt (file)"));
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

fn backup(source: &std::path::Path, repository: &std::path::Path) {
    assert!(
        traceback()
            .arg("backup")
            .arg(source)
            .arg("--repo")
            .arg(repository)
            .status()
            .expect("backup should execute")
            .success()
    );
}
