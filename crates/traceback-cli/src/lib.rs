use std::{env, error::Error, fs, io::Read, path::PathBuf, time::SystemTime};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use traceback_repo::{
    BlameError, CheckIssue, ChunkError, DiffEntry, DiffError, DoctorError, DoctorReport,
    ExplainError, ExplainReport, FileEntry, FileType, FindingLevel, HistoryError, IgnoreError,
    InitOptions, InitOutcome, MaintenanceError, ManifestError, ManifestSummary, OperationKind,
    RecoveryError, RepositoryError, RestoreError, SnapshotDiff, SnapshotManifest,
    StorageBlameEntry, StorageBlameReport, StorageError, StoreChunkOutcome, acquire_writer_lock,
    append_operation, apply_ignore_rules, blame_snapshot, check_repository, diff_snapshots,
    doctor_repository, explain_snapshot, gc_collect, gc_dry_run, list_manifests, prune_dry_run,
    prune_snapshots, recover_interrupted_writes, rehearse_restore, restore_snapshot,
    restore_snapshot_path, store_chunk, suggest_ignores, sync_repository_to_filesystem_remote,
    validate_repository, write_manifest,
};
use traceback_scan::{ScanOptions, ScannedEntry, ScannedFileType, scan};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "traceback")]
#[command(about = "Explainable backup and restore tool")]
#[command(version)]
pub struct Cli {
    /// Emit machine-readable JSON for supported commands.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress successful human-readable output.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Include additional human-readable detail.
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Disable progress messages for CI logs.
    #[arg(long, global = true)]
    pub no_progress: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a backup repository.
    Init {
        /// Repository directory to create.
        repo: PathBuf,

        /// Create an encrypted repository.
        #[arg(long)]
        encrypted: bool,

        /// Environment variable that supplies the repository passphrase.
        #[arg(long, default_value = "TRACEBACK_PASSPHRASE")]
        passphrase_env: String,
    },
    /// Create a snapshot from one or more source paths.
    Backup {
        /// Files or directories to include in the snapshot.
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Run a versioned backup policy file.
    Run {
        /// TOML policy file to execute.
        #[arg(long, default_value = "traceback.toml")]
        config: PathBuf,
    },
    /// List published snapshots.
    Snapshots {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Restore a snapshot or a path inside a snapshot.
    Restore {
        /// Snapshot ID or snapshot path expression.
        snapshot: String,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,

        /// Directory or file path to restore into.
        #[arg(long)]
        target: PathBuf,
    },
    /// Test restoring a snapshot into a temporary directory.
    Rehearse {
        /// Snapshot ID to rehearse.
        snapshot: String,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Verify repository integrity.
    Check {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Remove abandoned staging and temporary chunk artifacts.
    Recover {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Compare two snapshots.
    Diff {
        /// Older snapshot ID.
        old: String,

        /// Newer snapshot ID.
        new: String,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Explain what changed and caused storage growth in a snapshot.
    Explain {
        /// Snapshot ID or "latest".
        snapshot: String,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Attribute logical and physical storage to snapshot paths.
    BlameSize {
        /// Snapshot ID or "latest".
        snapshot: String,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Inspect repository reliability evidence and recommend actions.
    Doctor {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Report or remove unreferenced chunk files.
    Gc {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,

        /// Report what would be removed without deleting chunk files.
        #[arg(long)]
        dry_run: bool,

        /// Confirm deleting orphaned chunk files.
        #[arg(long)]
        yes: bool,
    },
    /// Plan or remove old snapshot manifests.
    Prune {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,

        /// Number of newest snapshots to retain.
        #[arg(long, default_value_t = 1)]
        keep_latest: usize,

        /// Report what would be removed without deleting manifests.
        #[arg(long)]
        dry_run: bool,

        /// Confirm deleting selected snapshot manifests.
        #[arg(long)]
        yes: bool,
    },
    /// Suggest or apply source ignore rules.
    Ignore {
        #[command(subcommand)]
        command: IgnoreCommand,
    },
    /// Push repository objects to a remote backend.
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum IgnoreCommand {
    /// Suggest likely dependency, build, cache, temporary, and generated paths.
    Suggest {
        /// Source tree to inspect.
        path: PathBuf,
    },
    /// Preview or append reviewed ignore rules.
    Apply {
        /// Source tree whose .tracebackignore file will be updated.
        path: PathBuf,

        /// Include all currently suggested rules.
        #[arg(long)]
        suggested: bool,

        /// Include a specific reviewed rule; may be repeated.
        #[arg(long = "rule")]
        rules: Vec<String>,

        /// Confirm writing the reviewed rules.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RemoteCommand {
    /// Push durable repository objects to a filesystem remote.
    Push {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,

        /// Filesystem remote path, optionally prefixed with file://.
        #[arg(long)]
        remote: String,
    },
}

pub fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let json = cli.json;
    let quiet = cli.quiet;
    let verbose = cli.verbose;
    let progress = !cli.no_progress && !quiet && !json;
    match cli.command {
        Command::Init {
            repo,
            encrypted,
            passphrase_env,
        } => match traceback_repo::init_repository_with_options(
            &repo,
            &InitOptions {
                encrypted,
                passphrase_env,
            },
        )? {
            InitOutcome::Created(config) => {
                println!(
                    "Initialized TraceBack repository {} at {}",
                    config.repository_id,
                    repo.display()
                );
            }
            InitOutcome::AlreadyInitialized(config) => {
                println!(
                    "TraceBack repository {} is already initialized at {}",
                    config.repository_id,
                    repo.display()
                );
            }
        },
        Command::Backup { paths, repo } => {
            emit_progress(progress, "scanning sources and writing snapshot");
            let verbose_repo = repo.clone();
            let verbose_sources = paths.clone();
            let result = run_backup(BackupRequest {
                paths,
                repo,
                policy_ignore_patterns: Vec::new(),
                fail_on_changed_file: false,
            })?;
            if !quiet {
                println!("Backup completed.");
                if verbose {
                    println!("Repository:           {}", verbose_repo.display());
                    println!(
                        "Sources:              {}",
                        verbose_sources
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                println!("Files scanned:        {}", result.files_scanned);
                println!("Logical size:         {} B", result.logical_bytes);
                println!("New data stored:      {} B", result.newly_stored_bytes);
                println!("Ignored paths:        {}", result.ignored_count);
                println!("Warnings:             {}", result.warning_count);
                println!("Snapshot ID:          {}", result.snapshot_id);
            }
        }
        Command::Run { config } => {
            let policy = read_policy(&config)?;
            let retention_keep_latest = policy.retention_keep_latest;
            emit_progress(progress, "running backup policy");
            let verbose_repo = policy.backup.repository.clone();
            let verbose_sources = policy.backup.sources.clone();
            let result = run_backup(BackupRequest {
                paths: policy.backup.sources,
                repo: policy.backup.repository,
                policy_ignore_patterns: policy.ignore_patterns,
                fail_on_changed_file: policy.fail_on_changed_file,
            })?;
            if !quiet {
                println!("Policy backup completed.");
                println!("Config:               {}", config.display());
                if verbose {
                    println!("Repository:           {}", verbose_repo.display());
                    println!(
                        "Sources:              {}",
                        verbose_sources
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                println!("Files scanned:        {}", result.files_scanned);
                println!("Logical size:         {} B", result.logical_bytes);
                println!("New data stored:      {} B", result.newly_stored_bytes);
                println!("Ignored paths:        {}", result.ignored_count);
                println!("Warnings:             {}", result.warning_count);
                if let Some(keep_latest) = retention_keep_latest {
                    println!("Retention keep latest: {keep_latest}");
                }
                println!("Snapshot ID:          {}", result.snapshot_id);
            }
        }
        Command::Snapshots { repo } => {
            validate_repository(&repo)?;
            let manifests = list_manifests(&repo)?;
            if json {
                let snapshots = manifests.iter().map(SnapshotJson::from).collect::<Vec<_>>();
                print_json(&SnapshotsJson { snapshots })?;
            } else if manifests.is_empty() {
                println!("No snapshots found.");
            } else {
                println!(
                    "{:<36}  {:<20}  {:>12}  {:>12}  Sources",
                    "ID", "Created", "Logical", "Stored"
                );
                for manifest in manifests {
                    println!(
                        "{:<36}  {:<20}  {:>12}  {:>12}  {}",
                        manifest.snapshot_id,
                        display_timestamp(&manifest.created_at),
                        format!("{} B", manifest.summary.logical_bytes),
                        format!("{} B", manifest.summary.newly_stored_bytes),
                        manifest.sources.join(", ")
                    );
                }
            }
        }
        Command::Restore {
            snapshot,
            repo,
            target,
        } => {
            validate_repository(&repo)?;
            let summary =
                if let Some((snapshot_id, selected_path)) = parse_restore_expression(&snapshot) {
                    restore_snapshot_path(&repo, snapshot_id, selected_path, &target)?
                } else {
                    restore_snapshot(&repo, &snapshot, &target)?
                };
            println!("Restore completed.");
            println!("Snapshot ID:          {snapshot}");
            println!("Files restored:       {}", summary.files);
            println!("Directories restored: {}", summary.directories);
            println!("Symlinks restored:    {}", summary.symlinks);
            println!("Bytes restored:       {} B", summary.bytes);
        }
        Command::Rehearse { snapshot, repo } => {
            validate_repository(&repo)?;
            let summary = match rehearse_restore(&repo, &snapshot) {
                Ok(summary) => {
                    append_operation(
                        &repo,
                        OperationKind::Rehearse,
                        Some(&snapshot),
                        true,
                        format!(
                            "verified {} files and {} bytes",
                            summary.files, summary.bytes
                        ),
                    )?;
                    summary
                }
                Err(error) => {
                    append_operation(
                        &repo,
                        OperationKind::Rehearse,
                        Some(&snapshot),
                        false,
                        error.to_string(),
                    )?;
                    return Err(error.into());
                }
            };
            println!("Restore rehearsal completed.");
            println!("Snapshot ID:          {snapshot}");
            println!("Files verified:       {}", summary.files);
            println!("Directories verified: {}", summary.directories);
            println!("Symlinks verified:    {}", summary.symlinks);
            println!("Bytes verified:       {} B", summary.bytes);
            println!("Result:               PASS");
        }
        Command::Check { repo } => {
            let report = check_repository(&repo);
            let passed = report.passed();
            append_operation(
                &repo,
                OperationKind::Check,
                None,
                passed,
                if passed {
                    format!(
                        "verified {} manifests and {} chunks",
                        report.manifests_checked, report.chunks_verified
                    )
                } else {
                    format!("{} integrity issue(s)", report.issues.len())
                },
            )?;
            if json {
                let issues = report
                    .issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                print_json(&CheckJson {
                    passed,
                    manifests_checked: report.manifests_checked,
                    chunks_verified: report.chunks_verified,
                    orphaned_chunks: report.orphaned_chunks,
                    staging_leftovers: report.abandoned_staging_entries,
                    temporary_chunk_files: report.temporary_chunk_files,
                    issues,
                })?;
            } else {
                println!("Repository check completed.");
                println!("Manifests checked:    {}", report.manifests_checked);
                println!("Chunks verified:      {}", report.chunks_verified);
                println!("Orphaned chunks:      {}", report.orphaned_chunks);
                println!("Staging leftovers:    {}", report.abandoned_staging_entries);
                println!("Temporary chunks:     {}", report.temporary_chunk_files);
                if passed {
                    println!("Result:               PASS");
                } else {
                    println!("Result:               FAIL");
                    for issue in &report.issues {
                        println!("Issue:                {}", display_check_issue(issue));
                    }
                }
            }
            if !passed {
                return Err("repository check failed".into());
            }
        }
        Command::Recover { repo } => {
            let report = recover_interrupted_writes(&repo)?;
            if json {
                print_json(&RecoveryJson {
                    staging_entries_removed: report.staging_entries_removed,
                    temporary_chunks_removed: report.temporary_chunks_removed,
                })?;
            } else {
                println!("Repository recovery completed.");
                println!(
                    "Staging entries removed: {}",
                    report.staging_entries_removed
                );
                println!(
                    "Temporary chunks removed: {}",
                    report.temporary_chunks_removed
                );
            }
        }
        Command::Diff { old, new, repo } => {
            validate_repository(&repo)?;
            let diff = diff_snapshots(&repo, &old, &new)?;
            if json {
                print_json(&DiffJson::from(&diff))?;
            } else {
                print_snapshot_diff(&diff);
            }
        }
        Command::Explain { snapshot, repo } => {
            validate_repository(&repo)?;
            let report = explain_snapshot(&repo, &snapshot)?;
            if json {
                print_json(&ExplainJson::from(&report))?;
            } else {
                print_explain_report(&report);
            }
        }
        Command::BlameSize { snapshot, repo } => {
            validate_repository(&repo)?;
            let report = blame_snapshot(&repo, &snapshot)?;
            if json {
                print_json(&BlameJson::from(&report))?;
            } else {
                print_blame_report(&report);
            }
        }
        Command::Doctor { repo } => {
            validate_repository(&repo)?;
            let report = doctor_repository(&repo)?;
            if json {
                print_json(&report)?;
            } else {
                print_doctor_report(&report);
            }
        }
        Command::Gc { repo, dry_run, yes } => {
            if !dry_run && !yes {
                return Err("gc requires --dry-run or --yes".into());
            }
            validate_repository(&repo)?;
            let report = if yes {
                gc_collect(&repo)?
            } else {
                gc_dry_run(&repo)?
            };
            if json {
                print_json(&report)?;
            } else {
                print_gc_report(&report);
            }
        }
        Command::Prune {
            repo,
            keep_latest,
            dry_run,
            yes,
        } => {
            if !dry_run && !yes {
                return Err("prune requires --dry-run or --yes".into());
            }
            validate_repository(&repo)?;
            let plan = if yes {
                prune_snapshots(&repo, keep_latest)?
            } else {
                prune_dry_run(&repo, keep_latest)?
            };
            if json {
                print_json(&plan)?;
            } else {
                print_prune_plan(&plan);
            }
        }
        Command::Ignore {
            command: IgnoreCommand::Suggest { path },
        } => {
            let suggestions = suggest_ignores(&path)?;
            if json {
                print_json(&suggestions)?;
            } else {
                println!("Suggested ignore rules:");
                if suggestions.is_empty() {
                    println!("No suggestions found.");
                }
                for suggestion in suggestions {
                    println!(
                        "{}  {} B across {} path(s) [{}]",
                        suggestion.rule,
                        suggestion.estimated_bytes,
                        suggestion.matched_paths,
                        suggestion.category
                    );
                }
            }
        }
        Command::Ignore {
            command:
                IgnoreCommand::Apply {
                    path,
                    suggested,
                    mut rules,
                    yes,
                },
        } => {
            if suggested {
                rules.extend(
                    suggest_ignores(&path)?
                        .into_iter()
                        .map(|suggestion| suggestion.rule),
                );
            }
            rules.sort();
            rules.dedup();
            if rules.is_empty() {
                return Err(IgnoreError::InvalidRule("no rules selected".to_owned()).into());
            }
            if !yes {
                if json {
                    print_json(&IgnoreApplyPreview {
                        applied: false,
                        path: ignore_file_path(&path),
                        rules,
                    })?;
                } else {
                    println!("Ignore apply preview; no file was changed.");
                    println!("Target: {}", ignore_file_path(&path).display());
                    for rule in rules {
                        println!("  {rule}");
                    }
                    println!("Re-run with --yes to apply these reviewed rules.");
                }
            } else {
                let report = apply_ignore_rules(&path, &rules)?;
                if json {
                    print_json(&report)?;
                } else {
                    println!("Ignore rules applied.");
                    println!("Target:               {}", report.path.display());
                    println!("Rules added:          {}", report.added.len());
                    println!("Existing rules skipped: {}", report.skipped_existing.len());
                    for rule in report.added {
                        println!("A {rule}");
                    }
                }
            }
        }
        Command::Remote {
            command: RemoteCommand::Push { repo, remote },
        } => {
            validate_repository(&repo)?;
            let remote = parse_filesystem_remote(&remote)?;
            let report = sync_repository_to_filesystem_remote(&repo, &remote)?;
            if json {
                print_json(&report)?;
            } else if !quiet {
                println!("Remote push completed.");
                if verbose {
                    println!("Repository:           {}", repo.display());
                    println!("Remote:               {}", remote.display());
                }
                println!("Copied files:         {}", report.copied_files);
                println!("Skipped files:        {}", report.skipped_files);
                println!("Copied bytes:         {} B", report.copied_bytes);
            }
        }
    }

    Ok(())
}

pub fn error_code(error: &(dyn Error + 'static)) -> &'static str {
    if let Some(error) = error.downcast_ref::<RepositoryError>() {
        return match error {
            RepositoryError::NotDirectory(_) => "repository_not_directory",
            RepositoryError::IncompatibleDirectory(_) => "repository_incompatible",
            RepositoryError::InvalidConfig { .. } => "repository_config_invalid",
            RepositoryError::UnsupportedConfig(_) => "repository_config_unsupported",
            RepositoryError::MissingDirectory(_) => "repository_layout_invalid",
            RepositoryError::Locked(_) => "repository_locked",
            RepositoryError::InvalidLock { .. } => "repository_lock_invalid",
            RepositoryError::SerializeConfig(_)
            | RepositoryError::FormatTimestamp(_)
            | RepositoryError::Io { .. } => "repository_io_error",
        };
    }
    if let Some(error) = error.downcast_ref::<ManifestError>() {
        return match error {
            ManifestError::InvalidSnapshotId(_) => "snapshot_id_invalid",
            ManifestError::AlreadyExists(_) => "snapshot_already_exists",
            ManifestError::Chunk { .. } => "snapshot_chunk_invalid",
            ManifestError::Io { .. } => "manifest_io_error",
            _ => "manifest_invalid",
        };
    }
    if let Some(error) = error.downcast_ref::<ChunkError>() {
        return match error {
            ChunkError::Io { .. } => "chunk_io_error",
            ChunkError::Compression(_) | ChunkError::Decompression { .. } => "chunk_codec_error",
            _ => "chunk_invalid",
        };
    }
    if let Some(error) = error.downcast_ref::<RestoreError>() {
        return match error {
            RestoreError::PathNotFound(_) => "restore_path_not_found",
            RestoreError::TargetExists(_) => "restore_target_exists",
            RestoreError::PathEscapesTarget(_) => "restore_path_unsafe",
            RestoreError::SymlinkTraversal(_) => "restore_symlink_traversal",
            RestoreError::UnsupportedSymlinkTarget { .. } => "restore_symlink_unsafe",
            RestoreError::HashMismatch { .. } | RestoreError::SizeMismatch { .. } => {
                "restore_verification_failed"
            }
            RestoreError::Manifest(_) | RestoreError::Chunk(_) => "restore_data_invalid",
            RestoreError::Io { .. } => "restore_io_error",
        };
    }
    if error.downcast_ref::<DiffError>().is_some() {
        return "diff_failed";
    }
    if let Some(error) = error.downcast_ref::<ExplainError>() {
        return match error {
            ExplainError::NoSnapshots => "snapshot_not_found",
            ExplainError::SnapshotNotFound(_) => "snapshot_not_found",
            ExplainError::Manifest(_) | ExplainError::Diff(_) => "explain_data_invalid",
            ExplainError::Chunk(_) => "explain_chunk_invalid",
        };
    }
    if let Some(error) = error.downcast_ref::<BlameError>() {
        return match error {
            BlameError::NoSnapshots | BlameError::SnapshotNotFound(_) => "snapshot_not_found",
            BlameError::Manifest(_) | BlameError::Accounting(_) => "blame_data_invalid",
        };
    }
    if error.downcast_ref::<HistoryError>().is_some() {
        return "history_error";
    }
    if error.downcast_ref::<DoctorError>().is_some() {
        return "doctor_failed";
    }
    if error.downcast_ref::<IgnoreError>().is_some() {
        return "ignore_scan_failed";
    }
    if error.downcast_ref::<MaintenanceError>().is_some() {
        return "maintenance_failed";
    }
    if let Some(error) = error.downcast_ref::<RecoveryError>() {
        return match error {
            RecoveryError::Repository(_) => "recovery_repository_error",
            RecoveryError::Io { .. } => "recovery_io_error",
        };
    }
    if let Some(error) = error.downcast_ref::<StorageError>() {
        return match error {
            StorageError::EscapesRoot(_) => "remote_path_invalid",
            StorageError::ExistingObjectDiffers(_) => "remote_conflict",
            StorageError::Io { .. } => "remote_io_error",
        };
    }

    "command_failed"
}

#[derive(Serialize)]
struct SnapshotsJson {
    snapshots: Vec<SnapshotJson>,
}

#[derive(Serialize)]
struct SnapshotJson {
    id: String,
    created_at: String,
    sources: Vec<String>,
    file_count: u64,
    logical_bytes: u64,
    newly_stored_bytes: u64,
}

impl From<&SnapshotManifest> for SnapshotJson {
    fn from(manifest: &SnapshotManifest) -> Self {
        Self {
            id: manifest.snapshot_id.clone(),
            created_at: manifest.created_at.clone(),
            sources: manifest.sources.clone(),
            file_count: manifest.summary.file_count,
            logical_bytes: manifest.summary.logical_bytes,
            newly_stored_bytes: manifest.summary.newly_stored_bytes,
        }
    }
}

#[derive(Serialize)]
struct CheckJson {
    passed: bool,
    manifests_checked: usize,
    chunks_verified: usize,
    orphaned_chunks: usize,
    staging_leftovers: usize,
    temporary_chunk_files: usize,
    issues: Vec<String>,
}

#[derive(Serialize)]
struct RecoveryJson {
    staging_entries_removed: usize,
    temporary_chunks_removed: usize,
}

#[derive(Serialize)]
struct IgnoreApplyPreview {
    applied: bool,
    path: PathBuf,
    rules: Vec<String>,
}

#[derive(Serialize)]
struct DiffJson {
    old_snapshot_id: String,
    new_snapshot_id: String,
    added: Vec<DiffEntryJson>,
    removed: Vec<DiffEntryJson>,
    modified: Vec<DiffEntryJson>,
    unchanged: usize,
}

#[derive(Serialize)]
struct DiffEntryJson {
    path: String,
    old_type: Option<FileType>,
    new_type: Option<FileType>,
    old_size: u64,
    new_size: u64,
    byte_delta: i128,
    type_changed: bool,
    content_changed: bool,
}

impl From<&DiffEntry> for DiffEntryJson {
    fn from(entry: &DiffEntry) -> Self {
        Self {
            path: entry.path.clone(),
            old_type: entry.old_type,
            new_type: entry.new_type,
            old_size: entry.old_size,
            new_size: entry.new_size,
            byte_delta: entry.byte_delta,
            type_changed: entry.type_changed,
            content_changed: entry.content_changed,
        }
    }
}

impl From<&SnapshotDiff> for DiffJson {
    fn from(diff: &SnapshotDiff) -> Self {
        Self {
            old_snapshot_id: diff.old_snapshot_id.clone(),
            new_snapshot_id: diff.new_snapshot_id.clone(),
            added: diff.added.iter().map(DiffEntryJson::from).collect(),
            removed: diff.removed.iter().map(DiffEntryJson::from).collect(),
            modified: diff.modified.iter().map(DiffEntryJson::from).collect(),
            unchanged: diff.unchanged,
        }
    }
}

#[derive(Serialize)]
struct ExplainJson {
    snapshot_id: String,
    previous_snapshot_id: Option<String>,
    added: usize,
    removed: usize,
    modified: usize,
    unchanged: usize,
    logical_bytes: u64,
    newly_stored_bytes: u64,
    new_chunk_bytes: u64,
    reused_chunk_bytes: u64,
    growth_contributors: Vec<GrowthContributorJson>,
}

#[derive(Serialize)]
struct GrowthContributorJson {
    path: String,
    new_chunk_bytes: u64,
}

impl From<&ExplainReport> for ExplainJson {
    fn from(report: &ExplainReport) -> Self {
        Self {
            snapshot_id: report.snapshot_id.clone(),
            previous_snapshot_id: report.previous_snapshot_id.clone(),
            added: report.added,
            removed: report.removed,
            modified: report.modified,
            unchanged: report.unchanged,
            logical_bytes: report.logical_bytes,
            newly_stored_bytes: report.newly_stored_bytes,
            new_chunk_bytes: report.new_chunk_bytes,
            reused_chunk_bytes: report.reused_chunk_bytes,
            growth_contributors: report
                .growth_contributors
                .iter()
                .map(|contributor| GrowthContributorJson {
                    path: contributor.path.clone(),
                    new_chunk_bytes: contributor.new_chunk_bytes,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct BlameJson {
    snapshot_id: String,
    accounting_method: String,
    logical_bytes: u64,
    unique_stored_bytes: u64,
    shared_stored_bytes: u64,
    reclaimable_stored_bytes: u64,
    entries: Vec<BlameEntryJson>,
}

#[derive(Serialize)]
struct BlameEntryJson {
    path: String,
    file_type: FileType,
    logical_bytes: u64,
    unique_stored_bytes: u64,
    shared_stored_bytes: u64,
    reclaimable_stored_bytes: u64,
}

impl From<&StorageBlameEntry> for BlameEntryJson {
    fn from(entry: &StorageBlameEntry) -> Self {
        Self {
            path: entry.path.clone(),
            file_type: entry.file_type,
            logical_bytes: entry.logical_bytes,
            unique_stored_bytes: entry.unique_stored_bytes,
            shared_stored_bytes: entry.shared_stored_bytes,
            reclaimable_stored_bytes: entry.reclaimable_stored_bytes,
        }
    }
}

impl From<&StorageBlameReport> for BlameJson {
    fn from(report: &StorageBlameReport) -> Self {
        Self {
            snapshot_id: report.snapshot_id.clone(),
            accounting_method: report.accounting_method.clone(),
            logical_bytes: report.logical_bytes,
            unique_stored_bytes: report.unique_stored_bytes,
            shared_stored_bytes: report.shared_stored_bytes,
            reclaimable_stored_bytes: report.reclaimable_stored_bytes,
            entries: report.entries.iter().map(BlameEntryJson::from).collect(),
        }
    }
}

fn print_json(value: &impl Serialize) -> Result<(), serde_json::Error> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn emit_progress(enabled: bool, message: &str) {
    if enabled {
        println!("Progress: {message}...");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BackupResult {
    snapshot_id: String,
    files_scanned: u64,
    logical_bytes: u64,
    newly_stored_bytes: u64,
    ignored_count: usize,
    warning_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BackupRequest {
    paths: Vec<PathBuf>,
    repo: PathBuf,
    policy_ignore_patterns: Vec<String>,
    fail_on_changed_file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedPolicy {
    backup: PolicyBackup,
    ignore_patterns: Vec<String>,
    fail_on_changed_file: bool,
    retention_keep_latest: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PolicyBackup {
    sources: Vec<PathBuf>,
    repository: PathBuf,
}

#[derive(Debug, Deserialize)]
struct PolicyToml {
    version: u32,
    backup: PolicyBackupToml,
    #[serde(default)]
    ignore: PolicyIgnoreToml,
    #[serde(default)]
    retention: PolicyRetentionToml,
}

#[derive(Debug, Deserialize)]
struct PolicyBackupToml {
    sources: Vec<String>,
    repository: String,
    #[serde(default)]
    changing_file_policy: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PolicyIgnoreToml {
    #[serde(default)]
    patterns: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PolicyRetentionToml {
    keep_latest: Option<usize>,
}

fn read_policy(path: &std::path::Path) -> Result<ResolvedPolicy, Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let policy: PolicyToml = toml::from_str(&contents)?;
    if policy.version != 1 {
        return Err(format!("policy version must be 1, found {}", policy.version).into());
    }
    if policy.backup.sources.is_empty() {
        return Err("policy backup.sources must include at least one path".into());
    }
    if policy.backup.repository.trim().is_empty() {
        return Err("policy backup.repository must not be empty".into());
    }
    let fail_on_changed_file = match policy
        .backup
        .changing_file_policy
        .as_deref()
        .unwrap_or("retry_then_warn")
    {
        "retry_then_warn" => false,
        "fail_fast" => true,
        other => {
            return Err(format!(
                "policy backup.changing_file_policy must be retry_then_warn or fail_fast, found {other}"
            )
            .into());
        }
    };
    let sources = policy
        .backup
        .sources
        .iter()
        .map(|source| expand_policy_path(source))
        .collect::<Result<Vec<_>, _>>()?;
    for pattern in &policy.ignore.patterns {
        if pattern.trim().is_empty() || pattern.contains('\0') {
            return Err("policy ignore.patterns must not contain empty or NUL rules".into());
        }
    }
    if policy.retention.keep_latest == Some(0) {
        return Err("policy retention.keep_latest must be greater than zero".into());
    }
    Ok(ResolvedPolicy {
        backup: PolicyBackup {
            sources,
            repository: expand_policy_path(&policy.backup.repository)?,
        },
        ignore_patterns: policy.ignore.patterns,
        fail_on_changed_file,
        retention_keep_latest: policy.retention.keep_latest,
    })
}

fn expand_policy_path(path: &str) -> Result<PathBuf, Box<dyn Error>> {
    let expanded = if path == "~" {
        env::var("USERPROFILE")
            .or_else(|_| env::var("HOME"))
            .map_err(|_| "cannot expand ~ without USERPROFILE or HOME")?
    } else if let Some(rest) = path.strip_prefix("~/") {
        let home = env::var("USERPROFILE")
            .or_else(|_| env::var("HOME"))
            .map_err(|_| "cannot expand ~ without USERPROFILE or HOME")?;
        format!("{home}/{rest}")
    } else if let Some(rest) = path.strip_prefix("${") {
        let Some((name, suffix)) = rest.split_once('}') else {
            return Err(format!("invalid environment expansion in path: {path}").into());
        };
        format!("{}{}", env::var(name)?, suffix)
    } else if let Some(rest) = path.strip_prefix('$') {
        let name_len = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .map(char::len_utf8)
            .sum::<usize>();
        if name_len == 0 {
            return Err(format!("invalid environment expansion in path: {path}").into());
        }
        let (name, suffix) = rest.split_at(name_len);
        format!("{}{}", env::var(name)?, suffix)
    } else {
        path.to_owned()
    };
    Ok(PathBuf::from(expanded))
}

struct StreamedFile {
    chunks: Vec<String>,
    newly_stored_bytes: u64,
    content_hash: String,
    size: u64,
}

fn run_backup(request: BackupRequest) -> Result<BackupResult, Box<dyn Error>> {
    let BackupRequest {
        paths,
        repo,
        policy_ignore_patterns,
        fail_on_changed_file,
    } = request;
    let config = validate_repository(&repo)?;
    let _lock = acquire_writer_lock(&repo)?;
    let mut ignore_patterns = read_ignore_patterns(&paths)?;
    ignore_patterns.extend(policy_ignore_patterns);
    let mut options = ScanOptions::new(paths);
    options.repository = Some(repo.clone());
    options.ignore_patterns = ignore_patterns;
    let inventory = scan(&options)?;

    let snapshot_id = format!("snap_{}", Uuid::new_v4().simple());
    let mut files = Vec::new();
    let mut file_count = 0_u64;
    let mut logical_bytes = 0_u64;
    let mut newly_stored_bytes = 0_u64;
    let mut warning_count = inventory.warnings.len();

    for entry in &inventory.entries {
        if entry.relative_path == "." && entry.file_type == ScannedFileType::Directory {
            continue;
        }

        let Some((file_entry, entry_newly_stored_bytes)) =
            build_file_entry(&repo, entry, config.chunk_size_bytes)?
        else {
            if fail_on_changed_file {
                return Err(format!(
                    "file changed repeatedly during backup: {}",
                    entry.path.display()
                )
                .into());
            }
            warning_count += 1;
            continue;
        };
        if file_entry.file_type == FileType::File {
            file_count += 1;
            logical_bytes = logical_bytes
                .checked_add(file_entry.size)
                .ok_or("logical byte count overflows u64")?;
            newly_stored_bytes = newly_stored_bytes
                .checked_add(entry_newly_stored_bytes)
                .ok_or("stored byte count overflows u64")?;
        }
        files.push(file_entry);
    }

    let manifest = SnapshotManifest {
        manifest_version: 0,
        snapshot_id: snapshot_id.clone(),
        state: "complete".to_owned(),
        created_at: OffsetDateTime::now_utc().format(&Rfc3339)?,
        sources: inventory
            .entries
            .iter()
            .map(|entry| portable_source_label(&entry.source))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        files,
        summary: ManifestSummary {
            file_count,
            logical_bytes,
            newly_stored_bytes,
        },
    };
    write_manifest(&repo, &manifest)?;

    Ok(BackupResult {
        snapshot_id,
        files_scanned: file_count,
        logical_bytes,
        newly_stored_bytes,
        ignored_count: inventory.ignored_count,
        warning_count,
    })
}

fn read_ignore_patterns(paths: &[PathBuf]) -> Result<Vec<String>, Box<dyn Error>> {
    let mut patterns = Vec::new();
    for path in paths {
        let ignore_path = if path.is_dir() {
            path.join(".tracebackignore")
        } else {
            path.parent()
                .map(|parent| parent.join(".tracebackignore"))
                .unwrap_or_else(|| PathBuf::from(".tracebackignore"))
        };
        if !ignore_path.exists() {
            continue;
        }

        let contents = fs::read_to_string(&ignore_path)?;
        patterns.extend(
            contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_owned),
        );
    }

    Ok(patterns)
}

fn build_file_entry(
    repo: &std::path::Path,
    entry: &ScannedEntry,
    chunk_size_bytes: u64,
) -> Result<Option<(FileEntry, u64)>, Box<dyn Error>> {
    let path = manifest_path(entry);
    match entry.file_type {
        ScannedFileType::Directory => Ok(Some((
            FileEntry {
                path,
                file_type: FileType::Directory,
                size: 0,
                modified_at: entry.modified_at.clone(),
                permissions: entry.permissions,
                content_hash: None,
                chunks: Vec::new(),
                symlink_target: None,
            },
            0,
        ))),
        ScannedFileType::File => stream_file_entry(repo, entry, path, chunk_size_bytes),
        ScannedFileType::Symlink => Ok(Some((
            FileEntry {
                path,
                file_type: FileType::Symlink,
                size: 0,
                modified_at: entry.modified_at.clone(),
                permissions: entry.permissions,
                content_hash: None,
                chunks: Vec::new(),
                symlink_target: entry
                    .symlink_target
                    .as_ref()
                    .map(|target| target.to_string_lossy().replace('\\', "/")),
            },
            0,
        ))),
    }
}

fn stream_file_entry(
    repo: &std::path::Path,
    entry: &ScannedEntry,
    manifest_path: String,
    chunk_size_bytes: u64,
) -> Result<Option<(FileEntry, u64)>, Box<dyn Error>> {
    let mut total_newly_stored_bytes = 0_u64;
    for attempt in 0..2 {
        let before = fs::metadata(&entry.path)?;
        let streamed = store_file_chunks(repo, &entry.path, chunk_size_bytes)?;
        maybe_mutate_streamed_file_for_test(&entry.path)?;
        total_newly_stored_bytes = total_newly_stored_bytes
            .checked_add(streamed.newly_stored_bytes)
            .ok_or("stored byte count overflows u64")?;
        let after = fs::metadata(&entry.path)?;
        if metadata_matches(&before, &after) {
            return Ok(Some((
                FileEntry {
                    path: manifest_path.clone(),
                    file_type: FileType::File,
                    size: streamed.size,
                    modified_at: metadata_modified_at(&after),
                    permissions: metadata_permissions(&after),
                    content_hash: Some(streamed.content_hash),
                    chunks: streamed.chunks,
                    symlink_target: None,
                },
                total_newly_stored_bytes,
            )));
        }
        if attempt == 0 {
            continue;
        }
    }

    Ok(None)
}

fn store_file_chunks(
    repo: &std::path::Path,
    path: &std::path::Path,
    chunk_size_bytes: u64,
) -> Result<StreamedFile, Box<dyn Error>> {
    let chunk_size = usize::try_from(chunk_size_bytes)?;
    let mut file = fs::File::open(path)?;
    let mut chunks = Vec::new();
    let mut newly_stored_bytes = 0_u64;
    let mut hasher = blake3::Hasher::new();
    let mut total_size = 0_u64;
    let mut buffer = vec![0_u8; chunk_size];
    loop {
        let mut filled = 0;
        while filled < buffer.len() {
            let read = file.read(&mut buffer[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == 0 {
            break;
        }
        let chunk = &buffer[..filled];
        hasher.update(chunk);
        total_size = total_size
            .checked_add(u64::try_from(filled)?)
            .ok_or("file size overflows u64")?;
        let outcome = store_chunk(repo, chunk)?;
        let hash = match outcome {
            StoreChunkOutcome::Stored(metadata) => {
                newly_stored_bytes = newly_stored_bytes
                    .checked_add(metadata.stored_size)
                    .ok_or("stored byte count overflows u64")?;
                metadata.hash
            }
            StoreChunkOutcome::AlreadyExists(metadata) => metadata.hash,
        };
        chunks.push(hash);
        if filled < buffer.len() {
            break;
        }
    }
    Ok(StreamedFile {
        chunks,
        newly_stored_bytes,
        content_hash: hasher.finalize().to_hex().to_string(),
        size: total_size,
    })
}

fn metadata_matches(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && before.file_type() == after.file_type()
}

fn metadata_modified_at(metadata: &fs::Metadata) -> Option<String> {
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    OffsetDateTime::from_unix_timestamp(i64::try_from(duration.as_secs()).ok()?)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

#[cfg(unix)]
fn metadata_permissions(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;

    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn metadata_permissions(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(not(test))]
fn maybe_mutate_streamed_file_for_test(_path: &std::path::Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
fn maybe_mutate_streamed_file_for_test(path: &std::path::Path) -> Result<(), std::io::Error> {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "changing.txt")
    {
        fs::write(path, format!("changed-{}", Uuid::new_v4()))?;
    }
    Ok(())
}

fn manifest_path(entry: &ScannedEntry) -> String {
    let label = portable_source_label(&entry.source);
    if entry.relative_path == "." {
        label
    } else {
        format!("{label}/{}", entry.relative_path)
    }
}

fn portable_source_label(source: &std::path::Path) -> String {
    source
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_manifest_segment)
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| "source".to_owned())
}

fn sanitize_manifest_segment(segment: &str) -> String {
    segment
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '\0' => '_',
            character => character,
        })
        .collect()
}

fn display_timestamp(timestamp: &str) -> String {
    timestamp
        .strip_suffix('Z')
        .unwrap_or(timestamp)
        .replace('T', " ")
}

fn display_check_issue(issue: &CheckIssue) -> String {
    issue.to_string()
}

fn parse_restore_expression(snapshot: &str) -> Option<(&str, &str)> {
    let (snapshot_id, selected_path) = snapshot.split_once(':')?;
    if snapshot_id.is_empty() || selected_path.is_empty() {
        return None;
    }
    Some((snapshot_id, selected_path))
}

fn parse_filesystem_remote(remote: &str) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = remote.strip_prefix("file://") {
        if path.is_empty() {
            return Err("file remote path cannot be empty".into());
        }
        Ok(PathBuf::from(path))
    } else if remote.contains("://") {
        Err("only file:// remotes are supported in this release".into())
    } else {
        Ok(PathBuf::from(remote))
    }
}

fn ignore_file_path(path: &std::path::Path) -> PathBuf {
    if path.is_dir() {
        path.join(".tracebackignore")
    } else {
        path.parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".tracebackignore")
    }
}

fn print_snapshot_diff(diff: &SnapshotDiff) {
    println!("Snapshot diff completed.");
    println!("Old snapshot:         {}", diff.old_snapshot_id);
    println!("New snapshot:         {}", diff.new_snapshot_id);
    println!("Added:                {}", diff.added.len());
    println!("Removed:              {}", diff.removed.len());
    println!("Modified:             {}", diff.modified.len());
    println!("Unchanged:            {}", diff.unchanged);
    if diff.changed_count() == 0 {
        println!("No path changes found.");
        return;
    }

    for entry in &diff.added {
        print_diff_entry("A", entry);
    }
    for entry in &diff.removed {
        print_diff_entry("R", entry);
    }
    for entry in &diff.modified {
        print_diff_entry("M", entry);
    }
}

fn print_diff_entry(prefix: &str, entry: &DiffEntry) {
    let old_type = entry.old_type.map(file_type_name).unwrap_or("-");
    let new_type = entry.new_type.map(file_type_name).unwrap_or("-");
    println!(
        "{prefix} {} [{old_type} -> {new_type}, {} -> {} bytes, {:+} bytes, content_changed={}]",
        entry.path, entry.old_size, entry.new_size, entry.byte_delta, entry.content_changed
    );
}

fn file_type_name(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Directory => "directory",
        FileType::File => "file",
        FileType::Symlink => "symlink",
    }
}

fn print_explain_report(report: &ExplainReport) {
    println!("Snapshot explanation completed.");
    println!("Snapshot:             {}", report.snapshot_id);
    println!(
        "Previous snapshot:    {}",
        report.previous_snapshot_id.as_deref().unwrap_or("none")
    );
    println!("Added paths:          {}", report.added);
    println!("Removed paths:        {}", report.removed);
    println!("Modified paths:       {}", report.modified);
    println!("Unchanged paths:      {}", report.unchanged);
    println!("Logical size:         {} B", report.logical_bytes);
    println!("New data stored:      {} B", report.newly_stored_bytes);
    println!("New chunk content:    {} B", report.new_chunk_bytes);
    println!("Reused chunk content: {} B", report.reused_chunk_bytes);
    if report.growth_contributors.is_empty() {
        println!("No new chunk contributors.");
        return;
    }
    println!("Major growth contributors:");
    for contributor in report.growth_contributors.iter().take(10) {
        println!(
            "  {}: {} B new chunk content",
            contributor.path, contributor.new_chunk_bytes
        );
    }
}

fn print_blame_report(report: &StorageBlameReport) {
    println!("Storage blame completed.");
    println!("Snapshot:             {}", report.snapshot_id);
    println!("Accounting method:    {}", report.accounting_method);
    println!("Logical size:         {} B", report.logical_bytes);
    println!("Unique stored data:   {} B", report.unique_stored_bytes);
    println!("Shared stored data:   {} B", report.shared_stored_bytes);
    println!(
        "Snapshot reclaimable: {} B",
        report.reclaimable_stored_bytes
    );
    println!("Largest contributors:");
    for entry in report.entries.iter().take(20) {
        println!(
            "  {} ({}) logical={} B unique={} B shared={} B reclaimable={} B",
            entry.path,
            file_type_name(entry.file_type),
            entry.logical_bytes,
            entry.unique_stored_bytes,
            entry.shared_stored_bytes,
            entry.reclaimable_stored_bytes
        );
    }
}

fn print_doctor_report(report: &DoctorReport) {
    println!("Repository doctor completed.");
    println!(
        "Reliability score:    {}/100 ({})",
        report.health_score, report.scoring_version
    );
    println!(
        "Latest snapshot:      {}",
        report.latest_snapshot_id.as_deref().unwrap_or("none")
    );
    println!(
        "Current integrity:    {}",
        if report.integrity_passed {
            "PASS"
        } else {
            "FAIL"
        }
    );
    for category in &report.score_categories {
        println!(
            "Score category:       {} = {:?} ({} points)",
            category.code, category.status, category.points
        );
    }
    for finding in &report.findings {
        let label = match finding.level {
            FindingLevel::Good => "GOOD",
            FindingLevel::Warning => "WARN",
            FindingLevel::Critical => "CRITICAL",
        };
        println!("[{label}] {}: {}", finding.code, finding.message);
        if let Some(recommendation) = &finding.recommendation {
            println!("  Action: {recommendation}");
        }
    }
}

fn print_gc_report(report: &traceback_repo::GcReport) {
    println!(
        "Garbage collection {}.",
        if report.dry_run {
            "dry run completed"
        } else {
            "completed"
        }
    );
    println!("Orphaned chunks:      {}", report.orphaned_chunks.len());
    println!("Reclaimable bytes:    {} B", report.reclaimable_bytes);
    for chunk in &report.orphaned_chunks {
        println!("O {} {} B", chunk.hash, chunk.bytes);
    }
}

fn print_prune_plan(plan: &traceback_repo::PrunePlan) {
    println!(
        "Prune {}.",
        if plan.dry_run {
            "dry run completed"
        } else {
            "completed"
        }
    );
    println!("Keep latest:          {}", plan.keep_latest);
    println!("Snapshots retained:   {}", plan.retained_snapshots.len());
    println!("Snapshots selected:   {}", plan.pruned_snapshots.len());
    for snapshot in &plan.pruned_snapshots {
        println!("P {snapshot}");
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use tempfile::tempdir;
    use traceback_repo::init_repository;
    use traceback_scan::{ScannedEntry, ScannedFileType};

    use super::{Cli, Command, stream_file_entry};

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_each_core_command() {
        let cases = [
            vec!["traceback", "init", "./repo"],
            vec!["traceback", "backup", "./source", "--repo", "./repo"],
            vec!["traceback", "snapshots", "--repo", "./repo"],
            vec![
                "traceback",
                "diff",
                "snap_old",
                "snap_new",
                "--repo",
                "./repo",
            ],
            vec![
                "traceback",
                "restore",
                "snap_001",
                "--repo",
                "./repo",
                "--target",
                "./restored",
            ],
            vec![
                "traceback",
                "restore",
                "snap_001:/source/file.txt",
                "--repo",
                "./repo",
                "--target",
                "./file.txt",
            ],
            vec!["traceback", "rehearse", "snap_001", "--repo", "./repo"],
            vec!["traceback", "check", "--repo", "./repo"],
            vec!["traceback", "recover", "--repo", "./repo"],
        ];

        for args in cases {
            Cli::try_parse_from(args).expect("core command should parse");
        }
    }

    #[test]
    fn backup_requires_at_least_one_source_path() {
        let error = Cli::try_parse_from(["traceback", "backup", "--repo", "./repo"])
            .expect_err("backup without a source path should fail");

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn command_variants_remain_available() {
        let cli = Cli::parse_from(["traceback", "check", "--repo", "./repo"]);

        assert!(matches!(cli.command, Command::Check { .. }));
    }

    #[test]
    fn streaming_reader_skips_file_that_changes_twice() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let source = temporary.path().join("source");
        std::fs::create_dir(&source).expect("source should be created");
        let path = source.join("changing.txt");
        std::fs::write(&path, "before").expect("file should be written");
        init_repository(&repository).expect("repository should initialize");
        let entry = ScannedEntry {
            source,
            path,
            relative_path: "changing.txt".to_owned(),
            file_type: ScannedFileType::File,
            size: 6,
            modified_at: None,
            permissions: None,
            symlink_target: None,
        };

        let result = stream_file_entry(
            &repository,
            &entry,
            "source/changing.txt".to_owned(),
            4 * 1024 * 1024,
        )
        .expect("changing file should be handled");

        assert!(result.is_none());
    }
}
