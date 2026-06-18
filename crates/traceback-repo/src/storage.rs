use std::{
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage path escapes repository root: {0}")]
    EscapesRoot(PathBuf),
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

    use super::{LocalRepositoryStorage, RepositoryStorage, StorageError, WriteOnceOutcome};

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
}
