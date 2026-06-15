use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use thiserror::Error;

use crate::{FileEntry, FileType, ManifestError, read_manifest};

#[derive(Debug, Error)]
pub enum DiffError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub old_snapshot_id: String,
    pub new_snapshot_id: String,
    pub added: Vec<DiffEntry>,
    pub removed: Vec<DiffEntry>,
    pub modified: Vec<DiffEntry>,
    pub unchanged: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEntry {
    pub path: String,
    pub old_type: Option<FileType>,
    pub new_type: Option<FileType>,
    pub old_size: u64,
    pub new_size: u64,
    pub byte_delta: i128,
    pub type_changed: bool,
    pub content_changed: bool,
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
            (None, Some(new)) => diff.added.push(diff_entry(path, None, Some(new))),
            (Some(old), None) => diff.removed.push(diff_entry(path, Some(old), None)),
            (Some(old), Some(new)) if entries_match(old, new) => diff.unchanged += 1,
            (Some(old), Some(new)) => diff.modified.push(diff_entry(path, Some(old), Some(new))),
            (None, None) => unreachable!("path came from at least one manifest"),
        }
    }

    Ok(diff)
}

fn diff_entry(path: String, old: Option<&FileEntry>, new: Option<&FileEntry>) -> DiffEntry {
    let old_size = old.map_or(0, |entry| entry.size);
    let new_size = new.map_or(0, |entry| entry.size);
    DiffEntry {
        path,
        old_type: old.map(|entry| entry.file_type),
        new_type: new.map(|entry| entry.file_type),
        old_size,
        new_size,
        byte_delta: i128::from(new_size) - i128::from(old_size),
        type_changed: old
            .zip(new)
            .is_some_and(|(old, new)| old.file_type != new.file_type),
        content_changed: old
            .zip(new)
            .is_some_and(|(old, new)| content_identity(old) != content_identity(new)),
    }
}

fn content_identity(entry: &FileEntry) -> (Option<&str>, Option<&str>) {
    (
        entry.content_hash.as_deref(),
        entry.symlink_target.as_deref(),
    )
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

        assert_eq!(diff.added[0].path, "source/added.txt");
        assert_eq!(diff.added[0].old_size, 0);
        assert_eq!(diff.added[0].new_size, 5);
        assert_eq!(diff.added[0].byte_delta, 5);
        assert_eq!(diff.removed[0].path, "source/removed.txt");
        assert_eq!(diff.removed[0].byte_delta, -7);
        assert_eq!(diff.modified[0].path, "source/modified.txt");
        assert_eq!(diff.modified[0].old_size, 3);
        assert_eq!(diff.modified[0].new_size, 3);
        assert!(diff.modified[0].content_changed);
        assert!(!diff.modified[0].type_changed);
        assert_eq!(diff.unchanged, 2);
        assert_eq!(diff.changed_count(), 3);
    }

    #[test]
    fn reports_type_changes() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        write_manifest(
            &repository,
            &manifest("snap_old", vec![file("source/item", "old")]),
        )
        .expect("old manifest should be written");
        write_manifest(
            &repository,
            &manifest("snap_new", vec![directory("source/item")]),
        )
        .expect("new manifest should be written");

        let diff =
            diff_snapshots(&repository, "snap_old", "snap_new").expect("snapshots should diff");

        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0].old_type, Some(FileType::File));
        assert_eq!(diff.modified[0].new_type, Some(FileType::Directory));
        assert_eq!(diff.modified[0].byte_delta, -3);
        assert!(diff.modified[0].type_changed);
        assert!(diff.modified[0].content_changed);
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
