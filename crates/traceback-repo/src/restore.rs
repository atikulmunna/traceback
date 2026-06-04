use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

use crate::{
    ChunkError, FileEntry, FileType, ManifestError, SnapshotManifest, read_chunk, read_manifest,
    validate_manifest, verify_manifest_chunks,
};

#[derive(Debug, Error)]
pub enum RestoreError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("chunk error: {0}")]
    Chunk(#[from] ChunkError),
    #[error("restore path escapes target: {0}")]
    PathEscapesTarget(String),
    #[error("restore target already exists: {0}")]
    TargetExists(PathBuf),
    #[error("restored file hash mismatch for {path}: expected {expected}, found {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("restored file size mismatch for {path}: expected {expected}, found {actual}")]
    SizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    #[error("unsupported symlink target for {path}: {target}")]
    UnsupportedSymlinkTarget { path: PathBuf, target: String },
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreSummary {
    pub directories: u64,
    pub files: u64,
    pub symlinks: u64,
    pub bytes: u64,
}

pub fn restore_snapshot(
    repository: &Path,
    snapshot_id: &str,
    target: &Path,
) -> Result<RestoreSummary, RestoreError> {
    let manifest = read_manifest(repository, snapshot_id)?;
    restore_manifest(repository, &manifest, target)
}

fn restore_manifest(
    repository: &Path,
    manifest: &SnapshotManifest,
    target: &Path,
) -> Result<RestoreSummary, RestoreError> {
    validate_manifest(manifest)?;
    verify_manifest_chunks(repository, manifest)?;

    let root = absolute_target(target)?;
    if root.exists() && !root.is_dir() {
        return Err(RestoreError::TargetExists(root));
    }
    fs::create_dir_all(&root).map_err(|source| io_error(&root, source))?;

    let mut entries = manifest.files.iter().collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        (
            match entry.file_type {
                FileType::Directory => 0_u8,
                FileType::File => 1,
                FileType::Symlink => 2,
            },
            entry.path.matches('/').count(),
        )
    });

    let mut summary = RestoreSummary {
        directories: 0,
        files: 0,
        symlinks: 0,
        bytes: 0,
    };

    for entry in entries {
        restore_entry(repository, &root, entry, &mut summary)?;
    }

    Ok(summary)
}

fn restore_entry(
    repository: &Path,
    root: &Path,
    entry: &FileEntry,
    summary: &mut RestoreSummary,
) -> Result<(), RestoreError> {
    let output = contained_output_path(root, &entry.path)?;
    match entry.file_type {
        FileType::Directory => {
            if output.exists() && !output.is_dir() {
                return Err(RestoreError::TargetExists(output));
            }
            fs::create_dir_all(&output).map_err(|source| io_error(&output, source))?;
            summary.directories += 1;
        }
        FileType::File => {
            if output.exists() {
                return Err(RestoreError::TargetExists(output));
            }
            let parent = output.parent().expect("output path has parent");
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
            let mut content = Vec::new();
            for hash in &entry.chunks {
                content.extend(read_chunk(repository, hash)?);
            }
            verify_file_content(&output, entry, &content)?;
            fs::write(&output, &content).map_err(|source| io_error(&output, source))?;
            let written = fs::read(&output).map_err(|source| io_error(&output, source))?;
            verify_file_content(&output, entry, &written)?;
            summary.files += 1;
            summary.bytes = summary
                .bytes
                .checked_add(entry.size)
                .expect("restored byte count should fit u64");
        }
        FileType::Symlink => {
            if output.exists() {
                return Err(RestoreError::TargetExists(output));
            }
            let parent = output.parent().expect("output path has parent");
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
            let target = entry.symlink_target.as_deref().ok_or_else(|| {
                RestoreError::UnsupportedSymlinkTarget {
                    path: output.clone(),
                    target: String::new(),
                }
            })?;
            validate_symlink_target(&output, target)?;
            create_symlink(target, &output)?;
            summary.symlinks += 1;
        }
    }

    Ok(())
}

fn verify_file_content(path: &Path, entry: &FileEntry, content: &[u8]) -> Result<(), RestoreError> {
    let actual_size =
        u64::try_from(content.len()).expect("usize fits into u64 on supported targets");
    if actual_size != entry.size {
        return Err(RestoreError::SizeMismatch {
            path: path.to_owned(),
            expected: entry.size,
            actual: actual_size,
        });
    }

    if let Some(expected) = &entry.content_hash {
        let actual = blake3::hash(content).to_hex().to_string();
        if &actual != expected {
            return Err(RestoreError::HashMismatch {
                path: path.to_owned(),
                expected: expected.clone(),
                actual,
            });
        }
    }

    Ok(())
}

fn contained_output_path(root: &Path, manifest_path: &str) -> Result<PathBuf, RestoreError> {
    let relative = lexical_relative_path(manifest_path)?;
    let output = root.join(relative);
    if !output.starts_with(root) {
        return Err(RestoreError::PathEscapesTarget(manifest_path.to_owned()));
    }
    Ok(output)
}

fn lexical_relative_path(path: &str) -> Result<PathBuf, RestoreError> {
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(segment) => relative.push(segment),
            _ => return Err(RestoreError::PathEscapesTarget(path.to_owned())),
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(RestoreError::PathEscapesTarget(path.to_owned()));
    }
    Ok(relative)
}

fn validate_symlink_target(path: &Path, target: &str) -> Result<(), RestoreError> {
    let target_path = Path::new(target);
    if target_path.is_absolute()
        || target_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RestoreError::UnsupportedSymlinkTarget {
            path: path.to_owned(),
            target: target.to_owned(),
        });
    }
    Ok(())
}

fn absolute_target(target: &Path) -> Result<PathBuf, RestoreError> {
    if target.is_absolute() {
        Ok(target.to_owned())
    } else {
        std::env::current_dir()
            .map(|current| current.join(target))
            .map_err(|source| io_error(target, source))
    }
}

#[cfg(unix)]
fn create_symlink(target: &str, output: &Path) -> Result<(), RestoreError> {
    std::os::unix::fs::symlink(target, output).map_err(|source| io_error(output, source))
}

#[cfg(windows)]
fn create_symlink(target: &str, output: &Path) -> Result<(), RestoreError> {
    std::os::windows::fs::symlink_file(target, output).map_err(|source| io_error(output, source))
}

fn io_error(path: &Path, source: io::Error) -> RestoreError {
    RestoreError::Io {
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

    use super::{RestoreError, restore_snapshot};

    #[test]
    fn restores_files_and_directories() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let target = temporary.path().join("restore");
        init_repository(&repository).expect("repository should initialize");
        let StoreChunkOutcome::Stored(chunk) =
            store_chunk(&repository, b"hello").expect("chunk should be stored")
        else {
            panic!("chunk should be newly stored");
        };
        write_manifest(&repository, &manifest("snap_restore", &chunk.hash))
            .expect("manifest should be written");

        let summary =
            restore_snapshot(&repository, "snap_restore", &target).expect("restore should work");

        assert_eq!(summary.files, 1);
        assert_eq!(summary.bytes, 5);
        assert_eq!(
            std::fs::read_to_string(target.join("source/file.txt"))
                .expect("restored file should be readable"),
            "hello"
        );
    }

    #[test]
    fn refuses_to_overwrite_existing_file() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let target = temporary.path().join("restore");
        init_repository(&repository).expect("repository should initialize");
        let StoreChunkOutcome::Stored(chunk) =
            store_chunk(&repository, b"hello").expect("chunk should be stored")
        else {
            panic!("chunk should be newly stored");
        };
        write_manifest(&repository, &manifest("snap_restore", &chunk.hash))
            .expect("manifest should be written");
        std::fs::create_dir_all(target.join("source")).expect("directory should be created");
        std::fs::write(target.join("source/file.txt"), "existing")
            .expect("existing file should be written");

        let error = restore_snapshot(&repository, "snap_restore", &target)
            .expect_err("restore should not overwrite");

        assert!(matches!(error, RestoreError::TargetExists(_)));
    }

    #[test]
    fn fails_when_required_chunk_is_missing() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let target = temporary.path().join("restore");
        init_repository(&repository).expect("repository should initialize");
        let missing_hash = "a".repeat(64);
        write_manifest(&repository, &manifest("snap_restore", &missing_hash))
            .expect("manifest should be written");

        let error = restore_snapshot(&repository, "snap_restore", &target)
            .expect_err("missing chunk should fail");

        assert!(matches!(error, RestoreError::Manifest(_)));
    }

    fn manifest(snapshot_id: &str, chunk_hash: &str) -> SnapshotManifest {
        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: snapshot_id.to_owned(),
            state: "complete".to_owned(),
            created_at: "2026-06-02T00:00:00Z".to_owned(),
            sources: vec!["source".to_owned()],
            files: vec![
                FileEntry {
                    path: "source".to_owned(),
                    file_type: FileType::Directory,
                    size: 0,
                    modified_at: None,
                    content_hash: None,
                    chunks: Vec::new(),
                    symlink_target: None,
                },
                FileEntry {
                    path: "source/file.txt".to_owned(),
                    file_type: FileType::File,
                    size: 5,
                    modified_at: None,
                    content_hash: Some(blake3::hash(b"hello").to_hex().to_string()),
                    chunks: vec![chunk_hash.to_owned()],
                    symlink_target: None,
                },
            ],
            summary: ManifestSummary {
                file_count: 1,
                logical_bytes: 5,
                newly_stored_bytes: 0,
            },
        }
    }
}
