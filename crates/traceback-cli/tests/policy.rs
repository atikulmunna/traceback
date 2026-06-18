use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn run_executes_valid_policy_backup_with_policy_ignores() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("source");
    let policy = temporary.path().join("traceback.toml");
    fs::create_dir_all(source.join("target")).expect("source should be created");
    fs::write(source.join("keep.txt"), "keep").expect("file should be written");
    fs::write(source.join("target/artifact.bin"), "ignored").expect("file should be written");
    init(&repository);
    fs::write(
        &policy,
        format!(
            r#"
version = 1

[backup]
sources = ["{}"]
repository = "{}"
changing_file_policy = "retry_then_warn"

[ignore]
patterns = ["target/"]

[retention]
keep_latest = 3
"#,
            source.display().to_string().replace('\\', "\\\\"),
            repository.display().to_string().replace('\\', "\\\\")
        ),
    )
    .expect("policy should be written");

    let output = traceback()
        .arg("run")
        .arg("--config")
        .arg(&policy)
        .output()
        .expect("policy run should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Policy backup completed."));
    assert!(stdout.contains("Files scanned:        1"));
    assert!(stdout.contains("Ignored paths:        1"));
    assert!(stdout.contains("Retention keep latest: 3"));
}

#[test]
fn run_rejects_invalid_policy_before_creating_manifest() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let policy = temporary.path().join("traceback.toml");
    init(&repository);
    fs::write(
        &policy,
        format!(
            r#"
version = 1

[backup]
sources = []
repository = "{}"
"#,
            repository.display().to_string().replace('\\', "\\\\")
        ),
    )
    .expect("policy should be written");

    let output = traceback()
        .arg("run")
        .arg("--config")
        .arg(&policy)
        .output()
        .expect("policy run should execute");

    assert!(!output.status.success());
    assert!(
        fs::read_dir(repository.join("snapshots"))
            .expect("snapshots should be readable")
            .next()
            .is_none()
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
