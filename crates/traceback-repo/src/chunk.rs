use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
};

use thiserror::Error;
use uuid::Uuid;

const CHUNK_MAGIC: &[u8; 8] = b"TBCHUNK\0";
const CHUNK_FORMAT_VERSION: u16 = 0;
const CHUNK_FLAGS: u16 = 0;
const HEADER_SIZE: usize = 8 + 2 + 2 + 8 + 8;
const COMPRESSION_LEVEL: i32 = 3;

#[derive(Debug, Error)]
pub enum ChunkError {
    #[error("invalid chunk hash: {0}")]
    InvalidHash(String),
    #[error("chunk file has invalid magic: {0}")]
    InvalidMagic(PathBuf),
    #[error("chunk file uses unsupported format version {version}: {path}")]
    UnsupportedVersion { path: PathBuf, version: u16 },
    #[error("chunk file uses unsupported flags {flags}: {path}")]
    UnsupportedFlags { path: PathBuf, flags: u16 },
    #[error("chunk file size does not match its header: {0}")]
    StoredSizeMismatch(PathBuf),
    #[error("decompressed chunk size does not match its header: {0}")]
    RawSizeMismatch(PathBuf),
    #[error("chunk hash mismatch for {path}: expected {expected}, found {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("compression error: {0}")]
    Compression(io::Error),
    #[error("decompression error at {path}: {source}")]
    Decompression { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkMetadata {
    pub hash: String,
    pub raw_size: u64,
    pub stored_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreChunkOutcome {
    Stored(ChunkMetadata),
    AlreadyExists(ChunkMetadata),
}

pub fn store_chunk(repository: &Path, content: &[u8]) -> Result<StoreChunkOutcome, ChunkError> {
    let hash = blake3::hash(content).to_hex().to_string();
    let chunk_path = chunk_path(repository, &hash)?;

    if chunk_path.exists() {
        return Ok(StoreChunkOutcome::AlreadyExists(read_verified_metadata(
            &chunk_path,
            &hash,
        )?));
    }

    let payload =
        zstd::stream::encode_all(content, COMPRESSION_LEVEL).map_err(ChunkError::Compression)?;
    let raw_size = u64::try_from(content.len()).expect("usize fits into u64 on supported targets");
    let stored_size =
        u64::try_from(payload.len()).expect("usize fits into u64 on supported targets");
    let metadata = ChunkMetadata {
        hash,
        raw_size,
        stored_size,
    };

    let parent = chunk_path
        .parent()
        .expect("chunk path always has a shard directory");
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;

    let temporary_path = parent.join(format!(".tmp-{}", Uuid::new_v4().simple()));
    write_chunk_file(&temporary_path, &metadata, &payload)?;
    if let Err(error) = read_chunk_file(&temporary_path, &metadata.hash) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    match fs::hard_link(&temporary_path, &chunk_path) {
        Ok(()) => {
            remove_temporary_file(&temporary_path)?;
            Ok(StoreChunkOutcome::Stored(metadata))
        }
        Err(_) if chunk_path.exists() => {
            remove_temporary_file(&temporary_path)?;
            Ok(StoreChunkOutcome::AlreadyExists(read_verified_metadata(
                &chunk_path,
                &metadata.hash,
            )?))
        }
        Err(_) => match fs::rename(&temporary_path, &chunk_path) {
            Ok(()) => Ok(StoreChunkOutcome::Stored(metadata)),
            Err(source) => {
                let _ = fs::remove_file(&temporary_path);
                Err(io_error(&chunk_path, source))
            }
        },
    }
}

pub fn read_chunk(repository: &Path, hash: &str) -> Result<Vec<u8>, ChunkError> {
    let path = chunk_path(repository, hash)?;
    read_chunk_file(&path, hash)
}

pub fn read_chunk_metadata(repository: &Path, hash: &str) -> Result<ChunkMetadata, ChunkError> {
    let path = chunk_path(repository, hash)?;
    read_verified_metadata(&path, hash)
}

fn read_chunk_file(path: &Path, hash: &str) -> Result<Vec<u8>, ChunkError> {
    let file = fs::read(path).map_err(|source| io_error(path, source))?;
    let (metadata, payload) = decode_header(path, hash, &file)?;
    let content =
        zstd::stream::decode_all(payload).map_err(|source| ChunkError::Decompression {
            path: path.to_owned(),
            source,
        })?;

    if u64::try_from(content.len()).expect("usize fits into u64 on supported targets")
        != metadata.raw_size
    {
        return Err(ChunkError::RawSizeMismatch(path.to_owned()));
    }

    let actual_hash = blake3::hash(&content).to_hex().to_string();
    if actual_hash != hash {
        return Err(ChunkError::HashMismatch {
            path: path.to_owned(),
            expected: hash.to_owned(),
            actual: actual_hash,
        });
    }

    Ok(content)
}

fn read_metadata(path: &Path, hash: &str) -> Result<ChunkMetadata, ChunkError> {
    let file = fs::read(path).map_err(|source| io_error(path, source))?;
    let (metadata, _) = decode_header(path, hash, &file)?;
    Ok(metadata)
}

fn read_verified_metadata(path: &Path, hash: &str) -> Result<ChunkMetadata, ChunkError> {
    read_chunk_file(path, hash)?;
    read_metadata(path, hash)
}

fn write_chunk_file(
    path: &Path,
    metadata: &ChunkMetadata,
    payload: &[u8],
) -> Result<(), ChunkError> {
    let mut file = fs::File::create(path).map_err(|source| io_error(path, source))?;
    file.write_all(CHUNK_MAGIC)
        .and_then(|()| file.write_all(&CHUNK_FORMAT_VERSION.to_le_bytes()))
        .and_then(|()| file.write_all(&CHUNK_FLAGS.to_le_bytes()))
        .and_then(|()| file.write_all(&metadata.raw_size.to_le_bytes()))
        .and_then(|()| file.write_all(&metadata.stored_size.to_le_bytes()))
        .and_then(|()| file.write_all(payload))
        .and_then(|()| file.sync_all())
        .map_err(|source| io_error(path, source))
}

fn decode_header<'a>(
    path: &Path,
    hash: &str,
    file: &'a [u8],
) -> Result<(ChunkMetadata, &'a [u8]), ChunkError> {
    if file.len() < HEADER_SIZE {
        return Err(ChunkError::StoredSizeMismatch(path.to_owned()));
    }

    let mut cursor = Cursor::new(file);
    let mut magic = [0; 8];
    cursor
        .read_exact(&mut magic)
        .map_err(|source| io_error(path, source))?;
    if &magic != CHUNK_MAGIC {
        return Err(ChunkError::InvalidMagic(path.to_owned()));
    }

    let version = read_u16(path, &mut cursor)?;
    if version != CHUNK_FORMAT_VERSION {
        return Err(ChunkError::UnsupportedVersion {
            path: path.to_owned(),
            version,
        });
    }

    let flags = read_u16(path, &mut cursor)?;
    if flags != CHUNK_FLAGS {
        return Err(ChunkError::UnsupportedFlags {
            path: path.to_owned(),
            flags,
        });
    }

    let raw_size = read_u64(path, &mut cursor)?;
    let stored_size = read_u64(path, &mut cursor)?;
    let payload = &file[HEADER_SIZE..];
    if u64::try_from(payload.len()).expect("usize fits into u64 on supported targets")
        != stored_size
    {
        return Err(ChunkError::StoredSizeMismatch(path.to_owned()));
    }

    Ok((
        ChunkMetadata {
            hash: hash.to_owned(),
            raw_size,
            stored_size,
        },
        payload,
    ))
}

fn read_u16(path: &Path, cursor: &mut Cursor<&[u8]>) -> Result<u16, ChunkError> {
    let mut bytes = [0; 2];
    cursor
        .read_exact(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u64(path: &Path, cursor: &mut Cursor<&[u8]>) -> Result<u64, ChunkError> {
    let mut bytes = [0; 8];
    cursor
        .read_exact(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    Ok(u64::from_le_bytes(bytes))
}

pub(crate) fn chunk_path(repository: &Path, hash: &str) -> Result<PathBuf, ChunkError> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ChunkError::InvalidHash(hash.to_owned()));
    }

    Ok(repository.join("chunks").join(&hash[..2]).join(hash))
}

fn remove_temporary_file(path: &Path) -> Result<(), ChunkError> {
    fs::remove_file(path).map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: io::Error) -> ChunkError {
    ChunkError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use tempfile::tempdir;

    use super::{ChunkError, HEADER_SIZE, StoreChunkOutcome, chunk_path, read_chunk, store_chunk};

    #[test]
    fn stores_and_reads_compressed_chunk() {
        let repository = tempdir().expect("temporary repository should be created");
        let content = b"repeat repeat repeat repeat repeat";

        let outcome = store_chunk(repository.path(), content).expect("chunk should be stored");
        let StoreChunkOutcome::Stored(metadata) = outcome else {
            panic!("first write should store chunk");
        };

        assert!(metadata.stored_size < metadata.raw_size);
        assert_eq!(
            read_chunk(repository.path(), &metadata.hash).expect("chunk should be readable"),
            content
        );
    }

    #[test]
    fn stores_duplicate_chunk_only_once() {
        let repository = tempdir().expect("temporary repository should be created");
        let content = b"same content";
        let first = store_chunk(repository.path(), content).expect("chunk should be stored");
        let second = store_chunk(repository.path(), content).expect("chunk should be reused");

        let StoreChunkOutcome::Stored(first_metadata) = first else {
            panic!("first write should store chunk");
        };
        assert_eq!(second, StoreChunkOutcome::AlreadyExists(first_metadata));
    }

    #[test]
    fn shards_chunk_by_first_two_hash_characters() {
        let repository = tempdir().expect("temporary repository should be created");
        let StoreChunkOutcome::Stored(metadata) =
            store_chunk(repository.path(), b"sharded").expect("chunk should be stored")
        else {
            panic!("first write should store chunk");
        };

        let path = chunk_path(repository.path(), &metadata.hash).expect("hash should be valid");

        assert_eq!(
            path.parent()
                .expect("chunk should have shard directory")
                .file_name()
                .expect("shard directory should have a name"),
            &metadata.hash[..2]
        );
    }

    #[test]
    fn rejects_invalid_hash() {
        let repository = tempdir().expect("temporary repository should be created");

        let error =
            read_chunk(repository.path(), "../invalid").expect_err("hash should be rejected");

        assert!(matches!(error, ChunkError::InvalidHash(_)));
    }

    #[test]
    fn rejects_uppercase_hash() {
        let repository = tempdir().expect("temporary repository should be created");
        let hash = "A".repeat(64);

        let error = read_chunk(repository.path(), &hash).expect_err("hash should be rejected");

        assert!(matches!(error, ChunkError::InvalidHash(_)));
    }

    #[test]
    fn detects_invalid_magic() {
        let repository = tempdir().expect("temporary repository should be created");
        let (path, metadata) = stored_chunk(repository.path());
        let mut file = fs::read(&path).expect("chunk file should be readable");
        file[0] = b'X';
        fs::write(&path, file).expect("chunk file should be overwritten");

        let error =
            read_chunk(repository.path(), &metadata.hash).expect_err("invalid magic should fail");

        assert!(matches!(error, ChunkError::InvalidMagic(_)));
    }

    #[test]
    fn detects_unsupported_version() {
        let repository = tempdir().expect("temporary repository should be created");
        let (path, metadata) = stored_chunk(repository.path());
        let mut file = fs::read(&path).expect("chunk file should be readable");
        file[8..10].copy_from_slice(&1_u16.to_le_bytes());
        fs::write(&path, file).expect("chunk file should be overwritten");

        let error = read_chunk(repository.path(), &metadata.hash).expect_err("version should fail");

        assert!(matches!(
            error,
            ChunkError::UnsupportedVersion { version: 1, .. }
        ));
    }

    #[test]
    fn detects_unsupported_flags() {
        let repository = tempdir().expect("temporary repository should be created");
        let (path, metadata) = stored_chunk(repository.path());
        let mut file = fs::read(&path).expect("chunk file should be readable");
        file[10..12].copy_from_slice(&1_u16.to_le_bytes());
        fs::write(&path, file).expect("chunk file should be overwritten");

        let error = read_chunk(repository.path(), &metadata.hash).expect_err("flags should fail");

        assert!(matches!(
            error,
            ChunkError::UnsupportedFlags { flags: 1, .. }
        ));
    }

    #[test]
    fn detects_raw_size_mismatch() {
        let repository = tempdir().expect("temporary repository should be created");
        let (path, metadata) = stored_chunk(repository.path());
        let mut file = fs::read(&path).expect("chunk file should be readable");
        file[12..20].copy_from_slice(&99_u64.to_le_bytes());
        fs::write(&path, file).expect("chunk file should be overwritten");

        let error =
            read_chunk(repository.path(), &metadata.hash).expect_err("raw size should fail");

        assert!(matches!(error, ChunkError::RawSizeMismatch(_)));
    }

    #[test]
    fn detects_hash_mismatch() {
        let repository = tempdir().expect("temporary repository should be created");
        let StoreChunkOutcome::Stored(first) =
            store_chunk(repository.path(), b"first").expect("first chunk should be stored")
        else {
            panic!("first write should store chunk");
        };
        let StoreChunkOutcome::Stored(second) =
            store_chunk(repository.path(), b"second").expect("second chunk should be stored")
        else {
            panic!("second write should store chunk");
        };
        let first_path = chunk_path(repository.path(), &first.hash).expect("hash should be valid");
        let second_path =
            chunk_path(repository.path(), &second.hash).expect("hash should be valid");
        fs::copy(second_path, first_path).expect("chunk should be replaced");

        let error = read_chunk(repository.path(), &first.hash).expect_err("hash should fail");

        assert!(matches!(error, ChunkError::HashMismatch { .. }));
    }

    #[test]
    fn refuses_to_reuse_corrupted_chunk() {
        let repository = tempdir().expect("temporary repository should be created");
        let content = b"payload";
        let (path, _) = stored_chunk(repository.path());
        let mut file = fs::read(&path).expect("chunk file should be readable");
        file[0] = b'X';
        fs::write(path, file).expect("chunk file should be overwritten");

        let error = store_chunk(repository.path(), content).expect_err("corruption should fail");

        assert!(matches!(error, ChunkError::InvalidMagic(_)));
    }

    #[test]
    fn detects_corrupted_payload() {
        let repository = tempdir().expect("temporary repository should be created");
        let StoreChunkOutcome::Stored(metadata) =
            store_chunk(repository.path(), b"payload").expect("chunk should be stored")
        else {
            panic!("first write should store chunk");
        };
        let path = chunk_path(repository.path(), &metadata.hash).expect("hash should be valid");
        let mut file = fs::read(&path).expect("chunk file should be readable");
        file[HEADER_SIZE] ^= 0xff;
        fs::write(&path, file).expect("chunk file should be overwritten");

        let error = read_chunk(repository.path(), &metadata.hash)
            .expect_err("corruption should be detected");

        assert!(matches!(error, ChunkError::Decompression { .. }));
    }

    #[test]
    fn detects_trailing_bytes() {
        let repository = tempdir().expect("temporary repository should be created");
        let StoreChunkOutcome::Stored(metadata) =
            store_chunk(repository.path(), b"payload").expect("chunk should be stored")
        else {
            panic!("first write should store chunk");
        };
        let path = chunk_path(repository.path(), &metadata.hash).expect("hash should be valid");
        let mut file = fs::read(&path).expect("chunk file should be readable");
        file.push(0);
        fs::write(&path, file).expect("chunk file should be overwritten");

        let error =
            read_chunk(repository.path(), &metadata.hash).expect_err("trailing byte should fail");

        assert!(matches!(error, ChunkError::StoredSizeMismatch(_)));
    }

    #[test]
    fn detects_truncated_header() {
        let repository = tempdir().expect("temporary repository should be created");
        let (path, metadata) = stored_chunk(repository.path());
        fs::write(&path, b"short").expect("chunk file should be overwritten");

        let error =
            read_chunk(repository.path(), &metadata.hash).expect_err("short header should fail");

        assert!(matches!(error, ChunkError::StoredSizeMismatch(_)));
    }

    #[test]
    fn successful_store_removes_temporary_file() {
        let repository = tempdir().expect("temporary repository should be created");
        let StoreChunkOutcome::Stored(metadata) =
            store_chunk(repository.path(), b"payload").expect("chunk should be stored")
        else {
            panic!("first write should store chunk");
        };
        let shard = chunk_path(repository.path(), &metadata.hash)
            .expect("hash should be valid")
            .parent()
            .expect("chunk should have shard directory")
            .to_owned();

        let entries = fs::read_dir(shard)
            .expect("shard should be readable")
            .collect::<Result<Vec<_>, _>>()
            .expect("shard entries should be readable");

        assert_eq!(entries.len(), 1);
    }

    fn stored_chunk(repository: &Path) -> (PathBuf, super::ChunkMetadata) {
        let StoreChunkOutcome::Stored(metadata) =
            store_chunk(repository, b"payload").expect("chunk should be stored")
        else {
            panic!("first write should store chunk");
        };
        let path = chunk_path(repository, &metadata.hash).expect("hash should be valid");
        (path, metadata)
    }
}
