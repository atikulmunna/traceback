use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use thiserror::Error;

use crate::{ChunkError, ManifestError, list_manifests, read_chunk_metadata};

#[derive(Debug, Error)]
pub enum AccountingError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("chunk error: {0}")]
    Chunk(#[from] ChunkError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkReferenceReport {
    pub chunks: Vec<ChunkReference>,
    pub unique_chunks: usize,
    pub raw_bytes: u64,
    pub stored_bytes: u64,
    pub single_snapshot_stored_bytes: u64,
    pub shared_stored_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkReference {
    pub hash: String,
    pub raw_size: u64,
    pub stored_size: u64,
    pub snapshot_references: usize,
    pub file_references: usize,
}

#[derive(Default)]
struct References {
    snapshots: BTreeSet<String>,
    files: BTreeSet<(String, String)>,
}

pub fn account_chunk_references(
    repository: &Path,
) -> Result<ChunkReferenceReport, AccountingError> {
    let manifests = list_manifests(repository)?;
    let mut references = BTreeMap::<String, References>::new();
    for manifest in manifests {
        for file in manifest.files {
            for hash in file.chunks {
                let reference = references.entry(hash).or_default();
                reference.snapshots.insert(manifest.snapshot_id.clone());
                reference
                    .files
                    .insert((manifest.snapshot_id.clone(), file.path.clone()));
            }
        }
    }

    let mut chunks = Vec::with_capacity(references.len());
    let mut raw_bytes = 0_u64;
    let mut stored_bytes = 0_u64;
    let mut single_snapshot_stored_bytes = 0_u64;
    let mut shared_stored_bytes = 0_u64;
    for (hash, references) in references {
        let metadata = read_chunk_metadata(repository, &hash)?;
        raw_bytes = raw_bytes
            .checked_add(metadata.raw_size)
            .expect("raw chunk bytes should fit u64");
        stored_bytes = stored_bytes
            .checked_add(metadata.stored_size)
            .expect("stored chunk bytes should fit u64");
        if references.snapshots.len() == 1 {
            single_snapshot_stored_bytes = single_snapshot_stored_bytes
                .checked_add(metadata.stored_size)
                .expect("stored chunk bytes should fit u64");
        } else {
            shared_stored_bytes = shared_stored_bytes
                .checked_add(metadata.stored_size)
                .expect("stored chunk bytes should fit u64");
        }
        chunks.push(ChunkReference {
            hash,
            raw_size: metadata.raw_size,
            stored_size: metadata.stored_size,
            snapshot_references: references.snapshots.len(),
            file_references: references.files.len(),
        });
    }

    Ok(ChunkReferenceReport {
        unique_chunks: chunks.len(),
        chunks,
        raw_bytes,
        stored_bytes,
        single_snapshot_stored_bytes,
        shared_stored_bytes,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{
        FileEntry, FileType, ManifestSummary, SnapshotManifest, StoreChunkOutcome, init_repository,
        store_chunk, write_manifest,
    };

    use super::account_chunk_references;

    #[test]
    fn counts_physical_chunks_once_across_snapshots_and_files() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let shared = stored_chunk(&repository, b"shared");
        let unique = stored_chunk(&repository, b"unique");
        write_manifest(
            &repository,
            &manifest(
                "snap_old",
                "2026-06-01T00:00:00Z",
                vec![file("a.txt", &shared, 6), file("copy.txt", &shared, 6)],
            ),
        )
        .expect("old manifest should write");
        write_manifest(
            &repository,
            &manifest(
                "snap_new",
                "2026-06-02T00:00:00Z",
                vec![file("a.txt", &shared, 6), file("b.txt", &unique, 6)],
            ),
        )
        .expect("new manifest should write");

        let report = account_chunk_references(&repository).expect("accounting should work");

        assert_eq!(report.unique_chunks, 2);
        assert_eq!(report.raw_bytes, 12);
        assert_eq!(
            report.stored_bytes,
            report.single_snapshot_stored_bytes + report.shared_stored_bytes
        );
        let shared = report
            .chunks
            .iter()
            .find(|chunk| chunk.hash == shared)
            .expect("shared chunk should be reported");
        assert_eq!(shared.snapshot_references, 2);
        assert_eq!(shared.file_references, 3);
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
