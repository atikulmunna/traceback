use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use thiserror::Error;

use crate::{ManifestError, RepositoryError, acquire_writer_lock, list_manifests};

#[derive(Debug, Error)]
pub enum MaintenanceError {
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GcReport {
    pub dry_run: bool,
    pub orphaned_chunks: Vec<OrphanedChunk>,
    pub reclaimable_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OrphanedChunk {
    pub hash: String,
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PrunePlan {
    pub dry_run: bool,
    pub keep_latest: usize,
    pub retained_snapshots: Vec<String>,
    pub pruned_snapshots: Vec<String>,
}

pub fn gc_dry_run(repository: &Path) -> Result<GcReport, MaintenanceError> {
    gc_plan(repository, true)
}

pub fn gc_collect(repository: &Path) -> Result<GcReport, MaintenanceError> {
    let _lock = acquire_writer_lock(repository)?;
    let report = gc_plan(repository, false)?;
    for chunk in &report.orphaned_chunks {
        fs::remove_file(&chunk.path).map_err(|source| io_error(&chunk.path, source))?;
    }
    Ok(report)
}

pub fn prune_dry_run(repository: &Path, keep_latest: usize) -> Result<PrunePlan, MaintenanceError> {
    prune_plan(repository, keep_latest, true)
}

pub fn prune_snapshots(
    repository: &Path,
    keep_latest: usize,
) -> Result<PrunePlan, MaintenanceError> {
    let _lock = acquire_writer_lock(repository)?;
    let plan = prune_plan(repository, keep_latest, false)?;
    for snapshot_id in &plan.pruned_snapshots {
        let path = repository
            .join("snapshots")
            .join(format!("{snapshot_id}.json"));
        fs::remove_file(&path).map_err(|source| io_error(&path, source))?;
    }
    Ok(plan)
}

fn gc_plan(repository: &Path, dry_run: bool) -> Result<GcReport, MaintenanceError> {
    let referenced = referenced_chunks(repository)?;
    let mut orphaned_chunks = Vec::new();
    let chunks = repository.join("chunks");
    for shard in fs::read_dir(&chunks).map_err(|source| io_error(&chunks, source))? {
        let shard = shard.map_err(|source| io_error(&chunks, source))?.path();
        if !shard.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&shard).map_err(|source| io_error(&shard, source))? {
            let path = entry.map_err(|source| io_error(&shard, source))?.path();
            if !path.is_file() {
                continue;
            }
            let Some(hash) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !is_chunk_hash(hash) || referenced.contains(hash) {
                continue;
            }
            let bytes = fs::metadata(&path)
                .map_err(|source| io_error(&path, source))?
                .len();
            orphaned_chunks.push(OrphanedChunk {
                hash: hash.to_owned(),
                path,
                bytes,
            });
        }
    }
    orphaned_chunks.sort_by(|left, right| left.hash.cmp(&right.hash));
    let reclaimable_bytes = orphaned_chunks.iter().map(|chunk| chunk.bytes).sum();
    Ok(GcReport {
        dry_run,
        orphaned_chunks,
        reclaimable_bytes,
    })
}

fn prune_plan(
    repository: &Path,
    keep_latest: usize,
    dry_run: bool,
) -> Result<PrunePlan, MaintenanceError> {
    let manifests = list_manifests(repository)?;
    let split_at = manifests.len().saturating_sub(keep_latest);
    let pruned_snapshots = manifests[..split_at]
        .iter()
        .map(|manifest| manifest.snapshot_id.clone())
        .collect();
    let retained_snapshots = manifests[split_at..]
        .iter()
        .map(|manifest| manifest.snapshot_id.clone())
        .collect();
    Ok(PrunePlan {
        dry_run,
        keep_latest,
        retained_snapshots,
        pruned_snapshots,
    })
}

fn referenced_chunks(repository: &Path) -> Result<BTreeSet<String>, MaintenanceError> {
    Ok(list_manifests(repository)?
        .into_iter()
        .flat_map(|manifest| manifest.files)
        .flat_map(|file| file.chunks)
        .collect())
}

fn is_chunk_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn io_error(path: &Path, source: io::Error) -> MaintenanceError {
    MaintenanceError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{
        FileEntry, FileType, ManifestSummary, SnapshotManifest, StoreChunkOutcome, init_repository,
        store_chunk, write_manifest,
    };

    use super::{gc_collect, gc_dry_run, prune_dry_run, prune_snapshots};

    #[test]
    fn gc_reports_and_removes_only_orphaned_chunks() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let referenced = stored_chunk(&repository, b"referenced");
        let orphaned = stored_chunk(&repository, b"orphaned");
        write_manifest(
            &repository,
            &manifest("snap_keep", "2026-06-01T00:00:00Z", &referenced),
        )
        .expect("manifest should write");
        let orphan_path = repository
            .join("chunks")
            .join(&orphaned[..2])
            .join(&orphaned);

        let dry_run = gc_dry_run(&repository).expect("dry run should work");

        assert!(dry_run.dry_run);
        assert_eq!(dry_run.orphaned_chunks.len(), 1);
        assert_eq!(dry_run.orphaned_chunks[0].hash, orphaned);
        assert!(orphan_path.exists());

        let collected = gc_collect(&repository).expect("collection should work");

        assert!(!collected.dry_run);
        assert_eq!(collected.orphaned_chunks.len(), 1);
        assert!(!orphan_path.exists());
        assert!(
            repository
                .join("chunks")
                .join(&referenced[..2])
                .join(referenced)
                .exists()
        );
    }

    #[test]
    fn prune_plans_and_removes_old_manifest_files_only() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let first = stored_chunk(&repository, b"first");
        let second = stored_chunk(&repository, b"second");
        write_manifest(
            &repository,
            &manifest("snap_old", "2026-06-01T00:00:00Z", &first),
        )
        .expect("old manifest should write");
        write_manifest(
            &repository,
            &manifest("snap_new", "2026-06-02T00:00:00Z", &second),
        )
        .expect("new manifest should write");

        let dry_run = prune_dry_run(&repository, 1).expect("prune dry run should work");

        assert!(dry_run.dry_run);
        assert_eq!(dry_run.pruned_snapshots, ["snap_old"]);
        assert!(repository.join("snapshots/snap_old.json").exists());

        let pruned = prune_snapshots(&repository, 1).expect("prune should work");

        assert!(!pruned.dry_run);
        assert!(!repository.join("snapshots/snap_old.json").exists());
        assert!(repository.join("snapshots/snap_new.json").exists());
        assert!(
            repository
                .join("chunks")
                .join(&first[..2])
                .join(first)
                .exists()
        );
    }

    fn stored_chunk(repository: &std::path::Path, content: &[u8]) -> String {
        match store_chunk(repository, content).expect("chunk should store") {
            StoreChunkOutcome::Stored(metadata) | StoreChunkOutcome::AlreadyExists(metadata) => {
                metadata.hash
            }
        }
    }

    fn manifest(snapshot_id: &str, created_at: &str, hash: &str) -> SnapshotManifest {
        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: snapshot_id.to_owned(),
            state: "complete".to_owned(),
            created_at: created_at.to_owned(),
            sources: vec!["source".to_owned()],
            files: vec![FileEntry {
                path: format!("source/{snapshot_id}.txt"),
                file_type: FileType::File,
                size: 1,
                modified_at: None,
                permissions: None,
                content_hash: Some(hash.to_owned()),
                chunks: vec![hash.to_owned()],
                symlink_target: None,
            }],
            summary: ManifestSummary {
                file_count: 1,
                logical_bytes: 1,
                newly_stored_bytes: 0,
            },
        }
    }
}
