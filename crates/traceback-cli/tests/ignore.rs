use std::{fs, process::Command};

use tempfile::tempdir;

fn traceback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_traceback"))
}

#[test]
fn ignore_suggest_reports_common_generated_paths_without_modifying_source() {
    let temporary = tempdir().expect("temporary directory should be created");
    let source = temporary.path().join("project");
    fs::create_dir_all(source.join("node_modules")).expect("directory should be created");
    fs::create_dir(source.join("target")).expect("directory should be created");
    fs::write(source.join("node_modules/package.js"), vec![0; 10]).expect("file should be written");
    fs::write(source.join("target/app.bin"), vec![0; 20]).expect("file should be written");
    fs::write(source.join("debug.tmp"), vec![0; 5]).expect("file should be written");

    let output = traceback()
        .arg("ignore")
        .arg("suggest")
        .arg(&source)
        .output()
        .expect("suggest should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("**/node_modules"));
    assert!(stdout.contains("**/target"));
    assert!(stdout.contains("*.tmp"));
    assert!(!source.join(".tracebackignore").exists());
}

#[test]
fn ignore_suggest_supports_json_output() {
    let temporary = tempdir().expect("temporary directory should be created");
    let source = temporary.path().join("project");
    fs::create_dir_all(source.join(".cache")).expect("directory should be created");
    fs::write(source.join(".cache/item"), vec![0; 8]).expect("file should be written");

    let output = traceback()
        .arg("--json")
        .arg("ignore")
        .arg("suggest")
        .arg(&source)
        .output()
        .expect("suggest should execute");

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output should be valid JSON");
    assert_eq!(json[0]["rule"], "**/.cache");
    assert_eq!(json[0]["category"], "cache");
    assert_eq!(json[0]["estimated_bytes"], 8);
    assert_eq!(json[0]["matched_paths"], 1);
    assert!(!source.join(".tracebackignore").exists());
}

#[test]
fn ignore_apply_previews_then_appends_reviewed_rules() {
    let temporary = tempdir().expect("temporary directory should be created");
    let source = temporary.path().join("project");
    fs::create_dir_all(source.join("target")).expect("directory should be created");
    fs::write(source.join("target/app.bin"), vec![0; 20]).expect("file should be written");
    fs::write(
        source.join(".tracebackignore"),
        "# keep this comment\n*.log\n",
    )
    .expect("ignore file should be written");

    let preview = traceback()
        .arg("ignore")
        .arg("apply")
        .arg(&source)
        .arg("--suggested")
        .output()
        .expect("preview should execute");

    assert!(preview.status.success());
    assert!(String::from_utf8_lossy(&preview.stdout).contains("no file was changed"));
    let before =
        fs::read_to_string(source.join(".tracebackignore")).expect("ignore file should read");
    assert_eq!(before, "# keep this comment\n*.log\n");

    let applied = traceback()
        .arg("ignore")
        .arg("apply")
        .arg(&source)
        .arg("--suggested")
        .arg("--yes")
        .output()
        .expect("apply should execute");

    assert!(applied.status.success());
    let contents =
        fs::read_to_string(source.join(".tracebackignore")).expect("ignore file should read");
    assert!(contents.starts_with("# keep this comment\n*.log\n"));
    assert!(contents.contains("**/target\n"));
}

#[test]
fn applied_rule_is_used_by_the_next_backup() {
    let temporary = tempdir().expect("temporary directory should be created");
    let repository = temporary.path().join("repo");
    let source = temporary.path().join("project");
    fs::create_dir_all(source.join("target")).expect("directory should be created");
    fs::write(source.join("target/app.bin"), vec![0; 20]).expect("file should be written");
    fs::write(source.join("keep.txt"), "keep").expect("file should be written");
    assert!(
        traceback()
            .arg("ignore")
            .arg("apply")
            .arg(&source)
            .arg("--rule")
            .arg("**/target")
            .arg("--yes")
            .status()
            .expect("apply should execute")
            .success()
    );
    assert!(
        traceback()
            .arg("init")
            .arg(&repository)
            .status()
            .expect("init should execute")
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
    let stdout = String::from_utf8_lossy(&backup.stdout);
    assert!(stdout.contains("Files scanned:        2"));
    assert!(stdout.contains("Ignored paths:        1"));
}
