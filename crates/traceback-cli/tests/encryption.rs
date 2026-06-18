use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn encrypted_repository_round_trips_chunks_with_passphrase() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    let target = temporary.path().join("restore");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("secret.txt"), "secret plaintext").expect("file should be written");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .arg("--encrypted")
            .arg("--passphrase-env")
            .arg("TRACEBACK_TEST_PASSPHRASE")
            .status()
            .expect("init should execute")
            .success()
    );

    let backup = traceback()
        .env("TRACEBACK_TEST_PASSPHRASE", "correct horse battery staple")
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");

    assert!(backup.status.success());
    let snapshot = snapshot_id_from_output(&String::from_utf8_lossy(&backup.stdout));
    let chunk_bytes = fs::read(first_chunk_file(&repository)).expect("chunk should be readable");
    assert!(
        !chunk_bytes
            .windows("secret plaintext".len())
            .any(|window| window == b"secret plaintext")
    );

    let restore = traceback()
        .env("TRACEBACK_TEST_PASSPHRASE", "correct horse battery staple")
        .arg("restore")
        .arg(&snapshot)
        .arg("--repo")
        .arg(&repository)
        .arg("--target")
        .arg(&target)
        .output()
        .expect("restore should execute");

    assert!(restore.status.success());
    assert_eq!(
        fs::read_to_string(target.join("source/secret.txt"))
            .expect("restored file should be readable"),
        "secret plaintext"
    );
}

#[test]
fn encrypted_repository_rejects_wrong_passphrase() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    let target = temporary.path().join("restore");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("secret.txt"), "secret plaintext").expect("file should be written");
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .arg("--encrypted")
            .arg("--passphrase-env")
            .arg("TRACEBACK_TEST_PASSPHRASE_WRONG")
            .status()
            .expect("init should execute")
            .success()
    );
    let backup = traceback()
        .env("TRACEBACK_TEST_PASSPHRASE_WRONG", "correct")
        .arg("backup")
        .arg(&source)
        .arg("--repo")
        .arg(&repository)
        .output()
        .expect("backup should execute");
    assert!(backup.status.success());
    let snapshot = snapshot_id_from_output(&String::from_utf8_lossy(&backup.stdout));

    let restore = traceback()
        .env("TRACEBACK_TEST_PASSPHRASE_WRONG", "incorrect")
        .arg("restore")
        .arg(&snapshot)
        .arg("--repo")
        .arg(&repository)
        .arg("--target")
        .arg(&target)
        .output()
        .expect("restore should execute");

    assert!(!restore.status.success());
    assert!(String::from_utf8_lossy(&restore.stderr).contains("decryption error"));
}

fn snapshot_id_from_output(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Snapshot ID:").map(str::trim))
        .expect("output should include snapshot ID")
        .to_owned()
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
