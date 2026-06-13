use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    ChunkError, ManifestError, RepositoryError, SnapshotManifest, chunk::chunk_path, read_chunk,
    validate_manifest, validate_repository,
};

#[derive(Debug, Error)]
pub enum CheckIssue {
    #[error("repository structure is invalid: {0}")]
    Repository(RepositoryError),
    #[error("manifest is invalid at {path}: {source}")]
    Manifest {
        path: PathBuf,
        source: ManifestError,
    },
    #[error("referenced chunk is missing or corrupt in {snapshot_id}: {chunk}: {source}")]
    ReferencedChunk {
        snapshot_id: String,
        chunk: String,
        source: ChunkError,
    },
    #[error("orphaned chunk is not referenced by any published snapshot: {0}")]
    OrphanedChunk(PathBuf),
    #[error("abandoned staging data found: {0}")]
    AbandonedStaging(PathBuf),
    #[error("abandoned temporary chunk found: {0}")]
    AbandonedTemporaryChunk(PathBuf),
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Default)]
pub struct CheckReport {
    pub manifests_checked: usize,
    pub chunks_verified: usize,
    pub orphaned_chunks: usize,
    pub abandoned_staging_entries: usize,
    pub temporary_chunk_files: usize,
    pub issues: Vec<CheckIssue>,
}

impl CheckReport {
    pub fn passed(&self) -> bool {
        self.issues.is_empty()
    }
}

pub fn check_repository(repository: &Path) -> CheckReport {
    let mut report = CheckReport::default();

    if let Err(error) = validate_repository(repository) {
        report.issues.push(CheckIssue::Repository(error));
        return report;
    }

    let mut referenced_chunks = BTreeSet::new();
    collect_manifest_issues(repository, &mut referenced_chunks, &mut report);
    verify_referenced_chunks(repository, &referenced_chunks, &mut report);
    collect_orphaned_chunks(repository, &referenced_chunks, &mut report);
    collect_abandoned_staging(repository, &mut report);

    report
}

fn collect_manifest_issues(
    repository: &Path,
    referenced_chunks: &mut BTreeSet<String>,
    report: &mut CheckReport,
) {
    let snapshots = repository.join("snapshots");
    let entries = match fs::read_dir(&snapshots) {
        Ok(entries) => entries,
        Err(source) => {
            report.issues.push(io_issue(&snapshots, source));
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => {
                report.issues.push(io_issue(&snapshots, source));
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }

        let json = match fs::read_to_string(&path) {
            Ok(json) => json,
            Err(source) => {
                report.issues.push(io_issue(&path, source));
                continue;
            }
        };
        let manifest: SnapshotManifest = match serde_json::from_str(&json) {
            Ok(manifest) => manifest,
            Err(source) => {
                report.issues.push(CheckIssue::Manifest {
                    path: path.clone(),
                    source: ManifestError::InvalidJson {
                        path: path.clone(),
                        source,
                    },
                });
                continue;
            }
        };
        if let Err(source) = validate_manifest(&manifest) {
            report.issues.push(CheckIssue::Manifest {
                path: path.clone(),
                source,
            });
            continue;
        }

        report.manifests_checked += 1;
        for file in &manifest.files {
            referenced_chunks.extend(file.chunks.iter().cloned());
        }
    }
}

fn verify_referenced_chunks(
    repository: &Path,
    referenced_chunks: &BTreeSet<String>,
    report: &mut CheckReport,
) {
    for chunk in referenced_chunks {
        match read_chunk(repository, chunk) {
            Ok(_) => report.chunks_verified += 1,
            Err(source) => report.issues.push(CheckIssue::ReferencedChunk {
                snapshot_id: snapshot_for_chunk(repository, chunk).unwrap_or_default(),
                chunk: chunk.clone(),
                source,
            }),
        }
    }
}

fn snapshot_for_chunk(repository: &Path, chunk: &str) -> Option<String> {
    let snapshots = fs::read_dir(repository.join("snapshots")).ok()?;
    for entry in snapshots.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let manifest: SnapshotManifest =
            serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        if manifest
            .files
            .iter()
            .any(|file| file.chunks.iter().any(|hash| hash == chunk))
        {
            return Some(manifest.snapshot_id);
        }
    }
    None
}

fn collect_orphaned_chunks(
    repository: &Path,
    referenced_chunks: &BTreeSet<String>,
    report: &mut CheckReport,
) {
    let chunks = repository.join("chunks");
    let shards = match fs::read_dir(&chunks) {
        Ok(shards) => shards,
        Err(source) => {
            report.issues.push(io_issue(&chunks, source));
            return;
        }
    };

    for shard in shards {
        let shard = match shard {
            Ok(shard) => shard.path(),
            Err(source) => {
                report.issues.push(io_issue(&chunks, source));
                continue;
            }
        };
        if !shard.is_dir() {
            continue;
        }
        let chunk_files = match fs::read_dir(&shard) {
            Ok(chunk_files) => chunk_files,
            Err(source) => {
                report.issues.push(io_issue(&shard, source));
                continue;
            }
        };
        for chunk_file in chunk_files {
            let chunk_file = match chunk_file {
                Ok(chunk_file) => chunk_file.path(),
                Err(source) => {
                    report.issues.push(io_issue(&shard, source));
                    continue;
                }
            };
            if !chunk_file.is_file() {
                continue;
            }
            let Some(hash) = chunk_file.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if hash.starts_with(".tmp-") {
                report.temporary_chunk_files += 1;
                report
                    .issues
                    .push(CheckIssue::AbandonedTemporaryChunk(chunk_file));
                continue;
            }
            if !referenced_chunks.contains(hash) {
                report.orphaned_chunks += 1;
                report.issues.push(CheckIssue::OrphanedChunk(chunk_file));
                continue;
            }
            if let Ok(expected_path) = chunk_path(repository, hash)
                && expected_path != chunk_file
            {
                report.issues.push(CheckIssue::OrphanedChunk(chunk_file));
            }
        }
    }
}

fn collect_abandoned_staging(repository: &Path, report: &mut CheckReport) {
    let staging = repository.join("staging");
    let entries = match fs::read_dir(&staging) {
        Ok(entries) => entries,
        Err(source) => {
            report.issues.push(io_issue(&staging, source));
            return;
        }
    };

    for entry in entries {
        match entry {
            Ok(entry) => {
                report.abandoned_staging_entries += 1;
                report
                    .issues
                    .push(CheckIssue::AbandonedStaging(entry.path()));
            }
            Err(source) => report.issues.push(io_issue(&staging, source)),
        }
    }
}

fn io_issue(path: &Path, source: io::Error) -> CheckIssue {
    CheckIssue::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{
        CheckIssue, FileEntry, FileType, ManifestSummary, SnapshotManifest, StoreChunkOutcome,
        check_repository, init_repository, store_chunk, write_manifest,
    };

    #[test]
    fn passes_for_valid_repository() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let StoreChunkOutcome::Stored(chunk) =
            store_chunk(&repository, b"hello").expect("chunk should be stored")
        else {
            panic!("chunk should be stored");
        };
        write_manifest(&repository, &manifest(&chunk.hash)).expect("manifest should be written");

        let report = check_repository(&repository);

        assert!(report.passed());
        assert_eq!(report.manifests_checked, 1);
        assert_eq!(report.chunks_verified, 1);
    }

    #[test]
    fn reports_missing_referenced_chunk() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        write_manifest(&repository, &manifest(&"a".repeat(64)))
            .expect("manifest should be written");

        let report = check_repository(&repository);

        assert!(!report.passed());
        assert!(
            report
                .issues
                .iter()
                .any(|issue| matches!(issue, CheckIssue::ReferencedChunk { .. }))
        );
    }

    #[test]
    fn reports_orphaned_chunk() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        store_chunk(&repository, b"orphan").expect("orphan chunk should be stored");

        let report = check_repository(&repository);

        assert!(!report.passed());
        assert_eq!(report.orphaned_chunks, 1);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| matches!(issue, CheckIssue::OrphanedChunk(_)))
        );
    }

    #[test]
    fn reports_abandoned_staging_data() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        std::fs::create_dir(repository.join("staging").join("leftover"))
            .expect("staging data should be created");

        let report = check_repository(&repository);

        assert!(!report.passed());
        assert_eq!(report.abandoned_staging_entries, 1);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| matches!(issue, CheckIssue::AbandonedStaging(_)))
        );
    }

    #[test]
    fn reports_abandoned_temporary_chunk() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let shard = repository.join("chunks").join("aa");
        std::fs::create_dir(&shard).expect("chunk shard should be created");
        std::fs::write(shard.join(".tmp-abandoned"), "temporary")
            .expect("temporary chunk should be written");

        let report = check_repository(&repository);

        assert!(!report.passed());
        assert_eq!(report.temporary_chunk_files, 1);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| matches!(issue, CheckIssue::AbandonedTemporaryChunk(_)))
        );
    }

    fn manifest(chunk_hash: &str) -> SnapshotManifest {
        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: "snap_check".to_owned(),
            state: "complete".to_owned(),
            created_at: "2026-06-02T00:00:00Z".to_owned(),
            sources: vec!["source".to_owned()],
            files: vec![FileEntry {
                path: "source/file.txt".to_owned(),
                file_type: FileType::File,
                size: 5,
                modified_at: None,
                permissions: None,
                content_hash: Some(blake3::hash(b"hello").to_hex().to_string()),
                chunks: vec![chunk_hash.to_owned()],
                symlink_target: None,
            }],
            summary: ManifestSummary {
                file_count: 1,
                logical_bytes: 5,
                newly_stored_bytes: 0,
            },
        }
    }
}
