use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use thiserror::Error;

use crate::{FileEntry, ManifestError, read_manifest};

#[derive(Debug, Error)]
pub enum DiffError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub old_snapshot_id: String,
    pub new_snapshot_id: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    pub unchanged: usize,
}

impl SnapshotDiff {
    pub fn changed_count(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }
}

pub fn diff_snapshots(
    repository: &Path,
    old_snapshot_id: &str,
    new_snapshot_id: &str,
) -> Result<SnapshotDiff, DiffError> {
    let old = read_manifest(repository, old_snapshot_id)?;
    let new = read_manifest(repository, new_snapshot_id)?;
    let old_entries = entries_by_path(&old.files);
    let new_entries = entries_by_path(&new.files);
    let paths = old_entries
        .keys()
        .chain(new_entries.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut diff = SnapshotDiff {
        old_snapshot_id: old.snapshot_id,
        new_snapshot_id: new.snapshot_id,
        added: Vec::new(),
        removed: Vec::new(),
        modified: Vec::new(),
        unchanged: 0,
    };

    for path in paths {
        match (old_entries.get(&path), new_entries.get(&path)) {
            (None, Some(_)) => diff.added.push(path),
            (Some(_), None) => diff.removed.push(path),
            (Some(old), Some(new)) if entries_match(old, new) => diff.unchanged += 1,
            (Some(_), Some(_)) => diff.modified.push(path),
            (None, None) => unreachable!("path came from at least one manifest"),
        }
    }

    Ok(diff)
}

fn entries_by_path(entries: &[FileEntry]) -> BTreeMap<String, &FileEntry> {
    entries
        .iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect()
}

fn entries_match(left: &FileEntry, right: &FileEntry) -> bool {
    left.file_type == right.file_type
        && left.size == right.size
        && left.content_hash == right.content_hash
        && left.chunks == right.chunks
        && left.symlink_target == right.symlink_target
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{
        FileEntry, FileType, ManifestSummary, SnapshotManifest, init_repository, write_manifest,
    };

    use super::diff_snapshots;

    #[test]
    fn classifies_added_removed_modified_and_unchanged_paths() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        write_manifest(&repository, &manifest("snap_old", old_entries()))
            .expect("old manifest should be written");
        write_manifest(&repository, &manifest("snap_new", new_entries()))
            .expect("new manifest should be written");

        let diff =
            diff_snapshots(&repository, "snap_old", "snap_new").expect("snapshots should diff");

        assert_eq!(diff.added, ["source/added.txt"]);
        assert_eq!(diff.removed, ["source/removed.txt"]);
        assert_eq!(diff.modified, ["source/modified.txt"]);
        assert_eq!(diff.unchanged, 2);
        assert_eq!(diff.changed_count(), 3);
    }

    fn old_entries() -> Vec<FileEntry> {
        vec![
            directory("source"),
            file("source/unchanged.txt", "same"),
            file("source/modified.txt", "old"),
            file("source/removed.txt", "removed"),
        ]
    }

    fn new_entries() -> Vec<FileEntry> {
        vec![
            directory("source"),
            file("source/unchanged.txt", "same"),
            file("source/modified.txt", "new"),
            file("source/added.txt", "added"),
        ]
    }

    fn manifest(snapshot_id: &str, files: Vec<FileEntry>) -> SnapshotManifest {
        let file_count = files
            .iter()
            .filter(|entry| entry.file_type == FileType::File)
            .count() as u64;
        let logical_bytes = files.iter().map(|entry| entry.size).sum();

        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: snapshot_id.to_owned(),
            state: "complete".to_owned(),
            created_at: "2026-06-02T00:00:00Z".to_owned(),
            sources: vec!["source".to_owned()],
            files,
            summary: ManifestSummary {
                file_count,
                logical_bytes,
                newly_stored_bytes: 0,
            },
        }
    }

    fn directory(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            file_type: FileType::Directory,
            size: 0,
            modified_at: None,
            permissions: None,
            content_hash: None,
            chunks: Vec::new(),
            symlink_target: None,
        }
    }

    fn file(path: &str, content: &str) -> FileEntry {
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        FileEntry {
            path: path.to_owned(),
            file_type: FileType::File,
            size: content.len() as u64,
            modified_at: None,
            permissions: None,
            content_hash: Some(hash.clone()),
            chunks: vec![hash],
            symlink_target: None,
        }
    }
}
