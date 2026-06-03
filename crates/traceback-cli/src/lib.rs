use std::{error::Error, fs, path::PathBuf};

use clap::{Parser, Subcommand};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use traceback_repo::{
    FileEntry, FileType, InitOutcome, ManifestSummary, SnapshotManifest, StoreChunkOutcome,
    init_repository, store_chunk, validate_repository, write_manifest,
};
use traceback_scan::{ScanOptions, ScannedEntry, ScannedFileType, scan};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "traceback")]
#[command(about = "Explainable backup and restore tool")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a backup repository.
    Init {
        /// Repository directory to create.
        repo: PathBuf,
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
    /// Verify repository integrity.
    Check {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
}

pub fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Command::Init { repo } => match init_repository(&repo)? {
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
            let result = run_backup(paths, repo)?;
            println!("Backup completed.");
            println!("Files scanned:        {}", result.files_scanned);
            println!("Logical size:         {} B", result.logical_bytes);
            println!("New data stored:      {} B", result.newly_stored_bytes);
            println!("Ignored paths:        {}", result.ignored_count);
            println!("Warnings:             {}", result.warning_count);
            println!("Snapshot ID:          {}", result.snapshot_id);
        }
        Command::Snapshots { repo } => {
            println!(
                "Snapshot listing is not implemented yet: {}",
                repo.display()
            );
        }
        Command::Restore {
            snapshot,
            repo,
            target,
        } => {
            println!(
                "Restore is not implemented yet: {snapshot} from {} -> {}",
                repo.display(),
                target.display()
            );
        }
        Command::Check { repo } => {
            println!(
                "Repository check is not implemented yet: {}",
                repo.display()
            );
        }
    }

    Ok(())
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

fn run_backup(paths: Vec<PathBuf>, repo: PathBuf) -> Result<BackupResult, Box<dyn Error>> {
    let config = validate_repository(&repo)?;
    let ignore_patterns = read_ignore_patterns(&paths)?;
    let mut options = ScanOptions::new(paths);
    options.repository = Some(repo.clone());
    options.ignore_patterns = ignore_patterns;
    let inventory = scan(&options)?;

    let snapshot_id = format!("snap_{}", Uuid::new_v4().simple());
    let mut files = Vec::new();
    let mut file_count = 0_u64;
    let mut logical_bytes = 0_u64;
    let mut newly_stored_bytes = 0_u64;

    for entry in &inventory.entries {
        if entry.relative_path == "." && entry.file_type == ScannedFileType::Directory {
            continue;
        }

        let (file_entry, entry_newly_stored_bytes) =
            build_file_entry(&repo, entry, config.chunk_size_bytes)?;
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
        warning_count: inventory.warnings.len(),
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
) -> Result<(FileEntry, u64), Box<dyn Error>> {
    let path = manifest_path(entry);
    match entry.file_type {
        ScannedFileType::Directory => Ok((
            FileEntry {
                path,
                file_type: FileType::Directory,
                size: 0,
                modified_at: entry.modified_at.clone(),
                content_hash: None,
                chunks: Vec::new(),
                symlink_target: None,
            },
            0,
        )),
        ScannedFileType::File => {
            let content = entry
                .content
                .as_deref()
                .ok_or("scanner did not provide file content")?;
            let (chunks, newly_stored_bytes) = store_file_chunks(repo, content, chunk_size_bytes)?;
            Ok((
                FileEntry {
                    path,
                    file_type: FileType::File,
                    size: entry.size,
                    modified_at: entry.modified_at.clone(),
                    content_hash: Some(blake3::hash(content).to_hex().to_string()),
                    chunks,
                    symlink_target: None,
                },
                newly_stored_bytes,
            ))
        }
        ScannedFileType::Symlink => Ok((
            FileEntry {
                path,
                file_type: FileType::Symlink,
                size: 0,
                modified_at: entry.modified_at.clone(),
                content_hash: None,
                chunks: Vec::new(),
                symlink_target: entry
                    .symlink_target
                    .as_ref()
                    .map(|target| target.to_string_lossy().replace('\\', "/")),
            },
            0,
        )),
    }
}

fn store_file_chunks(
    repo: &std::path::Path,
    content: &[u8],
    chunk_size_bytes: u64,
) -> Result<(Vec<String>, u64), Box<dyn Error>> {
    let chunk_size = usize::try_from(chunk_size_bytes)?;
    if content.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let mut chunks = Vec::new();
    let mut newly_stored_bytes = 0_u64;
    for chunk in content.chunks(chunk_size) {
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
    }
    Ok((chunks, newly_stored_bytes))
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

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, Command};

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
                "restore",
                "snap_001",
                "--repo",
                "./repo",
                "--target",
                "./restored",
            ],
            vec!["traceback", "check", "--repo", "./repo"],
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
}
