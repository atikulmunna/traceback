use std::{
    fs, io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{RepositoryError, acquire_writer_lock};

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryReport {
    pub staging_entries_removed: usize,
    pub temporary_chunks_removed: usize,
}

pub fn recover_interrupted_writes(repository: &Path) -> Result<RecoveryReport, RecoveryError> {
    let _lock = acquire_writer_lock(repository)?;
    let staging_entries_removed = clear_staging(repository)?;
    let temporary_chunks_removed = clear_temporary_chunks(repository)?;
    Ok(RecoveryReport {
        staging_entries_removed,
        temporary_chunks_removed,
    })
}

fn clear_staging(repository: &Path) -> Result<usize, RecoveryError> {
    let staging = repository.join("staging");
    let entries = fs::read_dir(&staging).map_err(|source| io_error(&staging, source))?;
    let mut removed = 0;
    for entry in entries {
        let path = entry.map_err(|source| io_error(&staging, source))?.path();
        remove_path(&path)?;
        removed += 1;
    }
    Ok(removed)
}

fn clear_temporary_chunks(repository: &Path) -> Result<usize, RecoveryError> {
    let chunks = repository.join("chunks");
    let shards = fs::read_dir(&chunks).map_err(|source| io_error(&chunks, source))?;
    let mut removed = 0;
    for shard in shards {
        let shard = shard.map_err(|source| io_error(&chunks, source))?.path();
        if !shard.is_dir() {
            continue;
        }
        let entries = fs::read_dir(&shard).map_err(|source| io_error(&shard, source))?;
        for entry in entries {
            let path = entry.map_err(|source| io_error(&shard, source))?.path();
            let is_temporary = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".tmp-"));
            if is_temporary {
                remove_path(&path)?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn remove_path(path: &Path) -> Result<(), RecoveryError> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|source| io_error(path, source))
    } else {
        fs::remove_file(path).map_err(|source| io_error(path, source))
    }
}

fn io_error(path: &Path, source: io::Error) -> RecoveryError {
    RecoveryError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{init_repository, store_chunk};

    use super::recover_interrupted_writes;

    #[test]
    fn removes_staging_and_temporary_chunks_only() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        std::fs::create_dir(repository.join("staging").join("abandoned"))
            .expect("staging entry should be created");
        let shard = repository.join("chunks").join("aa");
        std::fs::create_dir(&shard).expect("chunk shard should be created");
        std::fs::write(shard.join(".tmp-abandoned"), "temporary")
            .expect("temporary chunk should be written");
        let stored = store_chunk(&repository, b"published").expect("chunk should be stored");

        let report =
            recover_interrupted_writes(&repository).expect("recovery should complete safely");

        assert_eq!(report.staging_entries_removed, 1);
        assert_eq!(report.temporary_chunks_removed, 1);
        assert!(
            repository
                .join("staging")
                .read_dir()
                .unwrap()
                .next()
                .is_none()
        );
        let hash = match stored {
            crate::StoreChunkOutcome::Stored(metadata)
            | crate::StoreChunkOutcome::AlreadyExists(metadata) => metadata.hash,
        };
        assert!(
            repository
                .join("chunks")
                .join(&hash[..2])
                .join(hash)
                .exists()
        );
    }
}
