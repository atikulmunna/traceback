use std::{
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage path escapes repository root: {0}")]
    EscapesRoot(PathBuf),
    #[error("remote object already exists with different contents: {0}")]
    ExistingObjectDiffers(PathBuf),
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOnceOutcome {
    Stored,
    AlreadyExists,
}

pub trait RepositoryStorage {
    fn exists(&self, key: &Path) -> Result<bool, StorageError>;
    fn absolute_path(&self, key: &Path) -> Result<PathBuf, StorageError>;
    fn read(&self, key: &Path) -> Result<Vec<u8>, StorageError>;
    fn write_once_verified<F>(
        &self,
        key: &Path,
        contents: &[u8],
        verify: F,
    ) -> Result<WriteOnceOutcome, StorageError>
    where
        F: FnOnce(&Path) -> io::Result<()>;
}

#[derive(Debug, Clone)]
pub struct LocalRepositoryStorage {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteSyncReport {
    pub copied_files: usize,
    pub skipped_files: usize,
    pub copied_bytes: u64,
}

impl LocalRepositoryStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn checked_path(&self, key: &Path) -> Result<PathBuf, StorageError> {
        if key.is_absolute()
            || key
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(StorageError::EscapesRoot(key.to_owned()));
        }

        Ok(self.root.join(key))
    }
}

pub fn sync_repository_to_filesystem_remote(
    repository: &Path,
    remote: &Path,
) -> Result<RemoteSyncReport, StorageError> {
    let storage = LocalRepositoryStorage::new(remote);
    let mut report = RemoteSyncReport {
        copied_files: 0,
        skipped_files: 0,
        copied_bytes: 0,
    };

    copy_object(repository, &storage, Path::new("config.toml"), &mut report)?;
    copy_tree(repository, &storage, Path::new("chunks"), &mut report)?;
    copy_tree(repository, &storage, Path::new("snapshots"), &mut report)?;

    Ok(report)
}

impl RepositoryStorage for LocalRepositoryStorage {
    fn exists(&self, key: &Path) -> Result<bool, StorageError> {
        Ok(self.checked_path(key)?.exists())
    }

    fn absolute_path(&self, key: &Path) -> Result<PathBuf, StorageError> {
        self.checked_path(key)
    }

    fn read(&self, key: &Path) -> Result<Vec<u8>, StorageError> {
        let path = self.checked_path(key)?;
        fs::read(&path).map_err(|source| io_error(&path, source))
    }

    fn write_once_verified<F>(
        &self,
        key: &Path,
        contents: &[u8],
        verify: F,
    ) -> Result<WriteOnceOutcome, StorageError>
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        let path = self.checked_path(key)?;
        if path.exists() {
            return Ok(WriteOnceOutcome::AlreadyExists);
        }

        let parent = path.parent().expect("repository object path has a parent");
        fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;

        let temporary_path = parent.join(format!(".tmp-{}", Uuid::new_v4().simple()));
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)
                .map_err(|source| io_error(&temporary_path, source))?;
            file.write_all(contents)
                .and_then(|()| file.sync_all())
                .map_err(|source| {
                    let _ = fs::remove_file(&temporary_path);
                    io_error(&temporary_path, source)
                })?;
        }

        if let Err(source) = verify(&temporary_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(io_error(&temporary_path, source));
        }

        match fs::hard_link(&temporary_path, &path) {
            Ok(()) => {
                remove_temporary_file(&temporary_path)?;
                Ok(WriteOnceOutcome::Stored)
            }
            Err(_) if path.exists() => {
                remove_temporary_file(&temporary_path)?;
                Ok(WriteOnceOutcome::AlreadyExists)
            }
            Err(_) => match fs::rename(&temporary_path, &path) {
                Ok(()) => Ok(WriteOnceOutcome::Stored),
                Err(source) => {
                    let _ = fs::remove_file(&temporary_path);
                    Err(io_error(&path, source))
                }
            },
        }
    }
}

fn remove_temporary_file(path: &Path) -> Result<(), StorageError> {
    fs::remove_file(path).map_err(|source| io_error(path, source))
}

fn copy_tree(
    repository: &Path,
    storage: &LocalRepositoryStorage,
    key: &Path,
    report: &mut RemoteSyncReport,
) -> Result<(), StorageError> {
    let source = repository.join(key);
    let entries = fs::read_dir(&source).map_err(|error| io_error(&source, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error(&source, error))?;
        let file_name = entry.file_name();
        let child_key = key.join(&file_name);
        let file_type = entry
            .file_type()
            .map_err(|source| io_error(&entry.path(), source))?;
        if file_type.is_dir() {
            copy_tree(repository, storage, &child_key, report)?;
        } else if file_type.is_file() && !is_temporary_object(&file_name.to_string_lossy()) {
            copy_object(repository, storage, &child_key, report)?;
        }
    }

    Ok(())
}

fn copy_object(
    repository: &Path,
    storage: &LocalRepositoryStorage,
    key: &Path,
    report: &mut RemoteSyncReport,
) -> Result<(), StorageError> {
    let source = repository.join(key);
    let contents = fs::read(&source).map_err(|error| io_error(&source, error))?;
    if storage.exists(key)? {
        if storage.read(key)? == contents {
            report.skipped_files += 1;
            return Ok(());
        }
        return Err(StorageError::ExistingObjectDiffers(
            storage.absolute_path(key)?,
        ));
    }

    let byte_count =
        u64::try_from(contents.len()).expect("usize fits into u64 on supported targets");
    let outcome = storage.write_once_verified(key, &contents, |path| {
        let written = fs::read(path)?;
        if written == contents {
            Ok(())
        } else {
            Err(io::Error::other("written object did not verify"))
        }
    })?;
    match outcome {
        WriteOnceOutcome::Stored => {
            report.copied_files += 1;
            report.copied_bytes += byte_count;
        }
        WriteOnceOutcome::AlreadyExists => {
            report.skipped_files += 1;
        }
    }

    Ok(())
}

fn is_temporary_object(file_name: &str) -> bool {
    file_name.starts_with(".tmp-") || file_name.ends_with(".tmp")
}

fn io_error(path: &Path, source: io::Error) -> StorageError {
    StorageError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::Path};

    use tempfile::tempdir;

    use super::{
        LocalRepositoryStorage, RepositoryStorage, StorageError, WriteOnceOutcome,
        sync_repository_to_filesystem_remote,
    };

    #[test]
    fn write_once_verifies_and_publishes_local_object() {
        let root = tempdir().expect("temporary root should be created");
        let storage = LocalRepositoryStorage::new(root.path());

        let outcome = storage
            .write_once_verified(Path::new("chunks/aa/hash"), b"payload", |path| {
                let contents = std::fs::read(path)?;
                if contents == b"payload" {
                    Ok(())
                } else {
                    Err(io::Error::other("unexpected payload"))
                }
            })
            .expect("object should publish");

        assert_eq!(outcome, WriteOnceOutcome::Stored);
        assert_eq!(
            storage
                .read(Path::new("chunks/aa/hash"))
                .expect("object should read"),
            b"payload"
        );
    }

    #[test]
    fn write_once_reports_existing_without_overwriting() {
        let root = tempdir().expect("temporary root should be created");
        let storage = LocalRepositoryStorage::new(root.path());
        let key = Path::new("snapshots/snap.json");

        storage
            .write_once_verified(key, b"first", |_| Ok(()))
            .expect("initial object should publish");
        let outcome = storage
            .write_once_verified(key, b"second", |_| Ok(()))
            .expect("existing object should be reported");

        assert_eq!(outcome, WriteOnceOutcome::AlreadyExists);
        assert_eq!(storage.read(key).expect("object should read"), b"first");
    }

    #[test]
    fn rejects_paths_that_escape_repository_root() {
        let root = tempdir().expect("temporary root should be created");
        let storage = LocalRepositoryStorage::new(root.path());

        let error = storage
            .read(Path::new("../outside"))
            .expect_err("escaping path should fail");

        assert!(matches!(error, StorageError::EscapesRoot(_)));
    }

    #[test]
    fn filesystem_remote_sync_copies_durable_repository_objects() {
        let repository = tempdir().expect("repository root should be created");
        let remote = tempdir().expect("remote root should be created");
        std::fs::write(repository.path().join("config.toml"), "config")
            .expect("config should be written");
        std::fs::create_dir_all(repository.path().join("chunks/aa"))
            .expect("chunk shard should be created");
        std::fs::write(repository.path().join("chunks/aa/hash"), "chunk")
            .expect("chunk should be written");
        std::fs::write(
            repository.path().join("chunks/aa/.tmp-leftover"),
            "temporary",
        )
        .expect("temporary chunk should be written");
        std::fs::create_dir_all(repository.path().join("snapshots"))
            .expect("snapshots should be created");
        std::fs::write(repository.path().join("snapshots/snap.json"), "snapshot")
            .expect("snapshot should be written");

        let report = sync_repository_to_filesystem_remote(repository.path(), remote.path())
            .expect("sync should succeed");

        assert_eq!(report.copied_files, 3);
        assert_eq!(report.skipped_files, 0);
        assert!(remote.path().join("config.toml").is_file());
        assert!(remote.path().join("chunks/aa/hash").is_file());
        assert!(!remote.path().join("chunks/aa/.tmp-leftover").exists());
        assert!(remote.path().join("snapshots/snap.json").is_file());

        let second = sync_repository_to_filesystem_remote(repository.path(), remote.path())
            .expect("second sync should skip existing objects");
        assert_eq!(second.copied_files, 0);
        assert_eq!(second.skipped_files, 3);
    }

    #[test]
    fn filesystem_remote_sync_rejects_conflicting_existing_object() {
        let repository = tempdir().expect("repository root should be created");
        let remote = tempdir().expect("remote root should be created");
        std::fs::write(repository.path().join("config.toml"), "local")
            .expect("config should be written");
        std::fs::create_dir_all(repository.path().join("chunks"))
            .expect("chunks should be created");
        std::fs::create_dir_all(repository.path().join("snapshots"))
            .expect("snapshots should be created");
        std::fs::write(remote.path().join("config.toml"), "remote")
            .expect("remote config should be written");

        let error = sync_repository_to_filesystem_remote(repository.path(), remote.path())
            .expect_err("conflicting remote object should fail");

        assert!(matches!(error, StorageError::ExistingObjectDiffers(_)));
    }
}
