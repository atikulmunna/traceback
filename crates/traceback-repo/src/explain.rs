use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use thiserror::Error;

use crate::{
    ChunkError, DiffError, FileType, ManifestError, diff_snapshots, list_manifests, read_chunk,
};

#[derive(Debug, Error)]
pub enum ExplainError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("snapshot diff error: {0}")]
    Diff(#[from] DiffError),
    #[error("chunk error: {0}")]
    Chunk(#[from] ChunkError),
    #[error("repository has no published snapshots")]
    NoSnapshots,
    #[error("snapshot was not found: {0}")]
    SnapshotNotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainReport {
    pub snapshot_id: String,
    pub previous_snapshot_id: Option<String>,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub unchanged: usize,
    pub logical_bytes: u64,
    pub newly_stored_bytes: u64,
    pub new_chunk_bytes: u64,
    pub reused_chunk_bytes: u64,
    pub growth_contributors: Vec<GrowthContributor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthContributor {
    pub path: String,
    pub new_chunk_bytes: u64,
}

pub fn explain_snapshot(
    repository: &Path,
    snapshot_id: &str,
) -> Result<ExplainReport, ExplainError> {
    let manifests = list_manifests(repository)?;
    if manifests.is_empty() {
        return Err(ExplainError::NoSnapshots);
    }
    let index = if snapshot_id == "latest" {
        manifests.len() - 1
    } else {
        manifests
            .iter()
            .position(|manifest| manifest.snapshot_id == snapshot_id)
            .ok_or_else(|| ExplainError::SnapshotNotFound(snapshot_id.to_owned()))?
    };
    let snapshot = &manifests[index];
    let previous = index.checked_sub(1).map(|previous| &manifests[previous]);
    let previous_chunks = previous
        .map(|manifest| {
            manifest
                .files
                .iter()
                .flat_map(|entry| entry.chunks.iter().cloned())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();

    let mut unique_chunks = BTreeSet::new();
    let mut new_chunk_bytes = 0_u64;
    let mut reused_chunk_bytes = 0_u64;
    let mut contributors = BTreeMap::<String, u64>::new();
    let mut files = snapshot
        .files
        .iter()
        .filter(|entry| entry.file_type == FileType::File)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for file in files {
        for hash in &file.chunks {
            if !unique_chunks.insert(hash.clone()) {
                continue;
            }
            let size = u64::try_from(read_chunk(repository, hash)?.len())
                .expect("chunk size should fit u64");
            if previous_chunks.contains(hash) {
                reused_chunk_bytes = reused_chunk_bytes
                    .checked_add(size)
                    .expect("chunk byte count should fit u64");
            } else {
                new_chunk_bytes = new_chunk_bytes
                    .checked_add(size)
                    .expect("chunk byte count should fit u64");
                *contributors.entry(file.path.clone()).or_default() += size;
            }
        }
    }

    let (added, removed, modified, unchanged) = if let Some(previous) = previous {
        let diff = diff_snapshots(repository, &previous.snapshot_id, &snapshot.snapshot_id)?;
        (
            diff.added.len(),
            diff.removed.len(),
            diff.modified.len(),
            diff.unchanged,
        )
    } else {
        (snapshot.files.len(), 0, 0, 0)
    };
    let mut growth_contributors = contributors
        .into_iter()
        .map(|(path, new_chunk_bytes)| GrowthContributor {
            path,
            new_chunk_bytes,
        })
        .collect::<Vec<_>>();
    growth_contributors.sort_by(|left, right| {
        right
            .new_chunk_bytes
            .cmp(&left.new_chunk_bytes)
            .then_with(|| left.path.cmp(&right.path))
    });

    Ok(ExplainReport {
        snapshot_id: snapshot.snapshot_id.clone(),
        previous_snapshot_id: previous.map(|manifest| manifest.snapshot_id.clone()),
        added,
        removed,
        modified,
        unchanged,
        logical_bytes: snapshot.summary.logical_bytes,
        newly_stored_bytes: snapshot.summary.newly_stored_bytes,
        new_chunk_bytes,
        reused_chunk_bytes,
        growth_contributors,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{
        FileEntry, FileType, ManifestSummary, SnapshotManifest, StoreChunkOutcome, init_repository,
        store_chunk, write_manifest,
    };

    use super::explain_snapshot;

    #[test]
    fn explains_changes_reuse_and_growth_contributors() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let shared = stored_chunk(&repository, b"shared");
        let added = stored_chunk(&repository, b"new content");
        write_manifest(
            &repository,
            &manifest(
                "snap_old",
                "2026-06-01T00:00:00Z",
                vec![file("a.txt", &shared)],
            ),
        )
        .expect("old manifest should be written");
        write_manifest(
            &repository,
            &manifest(
                "snap_new",
                "2026-06-02T00:00:00Z",
                vec![file("a.txt", &shared), file("b.txt", &added)],
            ),
        )
        .expect("new manifest should be written");

        let report = explain_snapshot(&repository, "latest").expect("snapshot should explain");

        assert_eq!(report.snapshot_id, "snap_new");
        assert_eq!(report.previous_snapshot_id.as_deref(), Some("snap_old"));
        assert_eq!(report.added, 1);
        assert_eq!(report.modified, 0);
        assert_eq!(report.reused_chunk_bytes, 6);
        assert_eq!(report.new_chunk_bytes, 11);
        assert_eq!(report.growth_contributors[0].path, "b.txt");
        assert_eq!(report.growth_contributors[0].new_chunk_bytes, 11);
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

    fn file(path: &str, hash: &str) -> FileEntry {
        let size = if path == "a.txt" { 6 } else { 11 };
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
