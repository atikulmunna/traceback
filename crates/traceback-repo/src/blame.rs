use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use thiserror::Error;

use crate::{
    AccountingError, ChunkReference, FileType, ManifestError, account_chunk_references,
    list_manifests,
};

pub const BLAME_ACCOUNTING_METHOD: &str =
    "physical chunks are assigned once to the lexicographically first file in the snapshot";

#[derive(Debug, Error)]
pub enum BlameError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("chunk accounting error: {0}")]
    Accounting(#[from] AccountingError),
    #[error("repository has no published snapshots")]
    NoSnapshots,
    #[error("snapshot was not found: {0}")]
    SnapshotNotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageBlameReport {
    pub snapshot_id: String,
    pub accounting_method: String,
    pub logical_bytes: u64,
    pub unique_stored_bytes: u64,
    pub shared_stored_bytes: u64,
    pub reclaimable_stored_bytes: u64,
    pub entries: Vec<StorageBlameEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageBlameEntry {
    pub path: String,
    pub file_type: FileType,
    pub logical_bytes: u64,
    pub unique_stored_bytes: u64,
    pub shared_stored_bytes: u64,
    pub reclaimable_stored_bytes: u64,
}

pub fn blame_snapshot(
    repository: &Path,
    snapshot_id: &str,
) -> Result<StorageBlameReport, BlameError> {
    let manifests = list_manifests(repository)?;
    if manifests.is_empty() {
        return Err(BlameError::NoSnapshots);
    }
    let snapshot = if snapshot_id == "latest" {
        manifests.last().expect("manifests is not empty")
    } else {
        manifests
            .iter()
            .find(|manifest| manifest.snapshot_id == snapshot_id)
            .ok_or_else(|| BlameError::SnapshotNotFound(snapshot_id.to_owned()))?
    };
    let accounting = account_chunk_references(repository)?;
    let chunks = accounting
        .chunks
        .iter()
        .map(|chunk| (chunk.hash.as_str(), chunk))
        .collect::<BTreeMap<_, _>>();

    let mut files = snapshot
        .files
        .iter()
        .filter(|entry| entry.file_type == FileType::File)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut claimed = BTreeSet::new();
    let mut file_entries = Vec::with_capacity(files.len());
    for file in files {
        let mut unique_stored_bytes = 0_u64;
        let mut shared_stored_bytes = 0_u64;
        let mut reclaimable_stored_bytes = 0_u64;
        let mut file_chunks = BTreeSet::new();
        for hash in &file.chunks {
            if !file_chunks.insert(hash) {
                continue;
            }
            let chunk = chunks
                .get(hash.as_str())
                .expect("manifest chunks are present in accounting");
            if chunk_is_reclaimable_by_file(chunk, &snapshot.snapshot_id, &file.path) {
                reclaimable_stored_bytes += chunk.stored_size;
            }
            if !claimed.insert(hash) {
                continue;
            }
            if chunk.snapshot_references == 1 {
                unique_stored_bytes += chunk.stored_size;
            } else {
                shared_stored_bytes += chunk.stored_size;
            }
        }
        file_entries.push(StorageBlameEntry {
            path: file.path.clone(),
            file_type: FileType::File,
            logical_bytes: file.size,
            unique_stored_bytes,
            shared_stored_bytes,
            reclaimable_stored_bytes,
        });
    }

    let directories = directory_paths(&file_entries);
    let mut entries = file_entries.clone();
    for directory in directories {
        let prefix = format!("{directory}/");
        let descendants = file_entries
            .iter()
            .filter(|entry| entry.path.starts_with(&prefix))
            .collect::<Vec<_>>();
        let referenced_chunks = snapshot
            .files
            .iter()
            .filter(|file| file.file_type == FileType::File && file.path.starts_with(&prefix))
            .flat_map(|file| file.chunks.iter())
            .collect::<BTreeSet<_>>();
        let reclaimable_stored_bytes = referenced_chunks
            .into_iter()
            .filter_map(|hash| chunks.get(hash.as_str()))
            .filter(|chunk| {
                chunk_is_reclaimable_by_directory(chunk, &snapshot.snapshot_id, &prefix)
            })
            .map(|chunk| chunk.stored_size)
            .sum();
        entries.push(StorageBlameEntry {
            path: directory,
            file_type: FileType::Directory,
            logical_bytes: descendants.iter().map(|entry| entry.logical_bytes).sum(),
            unique_stored_bytes: descendants
                .iter()
                .map(|entry| entry.unique_stored_bytes)
                .sum(),
            shared_stored_bytes: descendants
                .iter()
                .map(|entry| entry.shared_stored_bytes)
                .sum(),
            reclaimable_stored_bytes,
        });
    }
    entries.sort_by(|left, right| {
        let left_stored = left.unique_stored_bytes + left.shared_stored_bytes;
        let right_stored = right.unique_stored_bytes + right.shared_stored_bytes;
        right_stored
            .cmp(&left_stored)
            .then_with(|| left.path.cmp(&right.path))
    });

    let unique_stored_bytes = file_entries
        .iter()
        .map(|entry| entry.unique_stored_bytes)
        .sum();
    let shared_stored_bytes = file_entries
        .iter()
        .map(|entry| entry.shared_stored_bytes)
        .sum();
    let reclaimable_stored_bytes = snapshot
        .files
        .iter()
        .flat_map(|file| file.chunks.iter())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|hash| chunks.get(hash.as_str()))
        .filter(|chunk| {
            chunk
                .references
                .iter()
                .all(|reference| reference.snapshot_id == snapshot.snapshot_id)
        })
        .map(|chunk| chunk.stored_size)
        .sum();

    Ok(StorageBlameReport {
        snapshot_id: snapshot.snapshot_id.clone(),
        accounting_method: BLAME_ACCOUNTING_METHOD.to_owned(),
        logical_bytes: snapshot.summary.logical_bytes,
        unique_stored_bytes,
        shared_stored_bytes,
        reclaimable_stored_bytes,
        entries,
    })
}

fn directory_paths(files: &[StorageBlameEntry]) -> BTreeSet<String> {
    let mut directories = BTreeSet::new();
    for file in files {
        let segments = file.path.split('/').collect::<Vec<_>>();
        for length in 1..segments.len() {
            directories.insert(segments[..length].join("/"));
        }
    }
    directories
}

fn chunk_is_reclaimable_by_file(chunk: &ChunkReference, snapshot_id: &str, path: &str) -> bool {
    chunk
        .references
        .iter()
        .all(|reference| reference.snapshot_id == snapshot_id && reference.path == path)
}

fn chunk_is_reclaimable_by_directory(
    chunk: &ChunkReference,
    snapshot_id: &str,
    prefix: &str,
) -> bool {
    chunk
        .references
        .iter()
        .all(|reference| reference.snapshot_id == snapshot_id && reference.path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{
        FileEntry, FileType, ManifestSummary, SnapshotManifest, StoreChunkOutcome, init_repository,
        store_chunk, write_manifest,
    };

    use super::blame_snapshot;

    #[test]
    fn attributes_unique_shared_and_directory_reclaimable_bytes() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let shared = stored_chunk(&repository, b"shared");
        let directory_shared = stored_chunk(&repository, b"inside");
        write_manifest(
            &repository,
            &manifest(
                "snap_old",
                "2026-06-01T00:00:00Z",
                vec![file("source/old.txt", &shared, 6)],
            ),
        )
        .expect("old manifest should write");
        write_manifest(
            &repository,
            &manifest(
                "snap_new",
                "2026-06-02T00:00:00Z",
                vec![
                    file("source/shared.txt", &shared, 6),
                    file("source/dir/a.txt", &directory_shared, 6),
                    file("source/dir/b.txt", &directory_shared, 6),
                ],
            ),
        )
        .expect("new manifest should write");

        let report = blame_snapshot(&repository, "latest").expect("blame should work");

        assert!(report.unique_stored_bytes > 0);
        assert!(report.shared_stored_bytes > 0);
        assert_eq!(
            report.unique_stored_bytes + report.shared_stored_bytes,
            report
                .entries
                .iter()
                .filter(|entry| entry.file_type == FileType::File)
                .map(|entry| entry.unique_stored_bytes + entry.shared_stored_bytes)
                .sum::<u64>()
        );
        let directory = report
            .entries
            .iter()
            .find(|entry| entry.path == "source/dir")
            .expect("directory should be reported");
        assert!(directory.reclaimable_stored_bytes > 0);
        let files = report
            .entries
            .iter()
            .filter(|entry| entry.path.starts_with("source/dir/"))
            .collect::<Vec<_>>();
        assert!(
            files
                .iter()
                .all(|entry| entry.reclaimable_stored_bytes == 0)
        );
    }

    fn stored_chunk(repository: &std::path::Path, content: &[u8]) -> String {
        match store_chunk(repository, content).expect("chunk should store") {
            StoreChunkOutcome::Stored(metadata) | StoreChunkOutcome::AlreadyExists(metadata) => {
                metadata.hash
            }
        }
    }

    fn manifest(snapshot_id: &str, created_at: &str, files: Vec<FileEntry>) -> SnapshotManifest {
        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: snapshot_id.to_owned(),
            state: "complete".to_owned(),
            created_at: created_at.to_owned(),
            sources: vec!["source".to_owned()],
            summary: ManifestSummary {
                file_count: files.len() as u64,
                logical_bytes: files.iter().map(|file| file.size).sum(),
                newly_stored_bytes: 0,
            },
            files,
        }
    }

    fn file(path: &str, hash: &str, size: u64) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            file_type: FileType::File,
            size,
            modified_at: None,
            permissions: None,
            content_hash: Some(hash.to_owned()),
            chunks: vec![hash.to_owned()],
            symlink_target: None,
        }
    }
}
