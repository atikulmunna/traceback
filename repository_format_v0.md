# TraceBack Repository Format

**Format Version:** 0  
**Status:** Initial implementation specification  
**Scope:** Local repository format for the `0.1` vertical slice  

## 1. Design Goals

The initial repository format shall be simple, inspectable, and safe to implement.

- Snapshot manifests shall be readable versioned JSON.
- Chunks shall be immutable files addressed by BLAKE3 hash.
- The chunk index shall be derived data. It may be rebuilt from stored chunks and published manifests.
- A snapshot becomes visible only when its completed manifest is atomically published.
- Early implementation shall optimize for correctness and clear recovery behavior before storage-layout optimization.

## 2. Repository Layout

```text
repo/
  config.toml
  chunks/
    ab/
      abcdef...              # Full lowercase BLAKE3 hash
  snapshots/
    snap_<id>.json           # Published manifests only
  indexes/
    chunks.json              # Optional derived cache
  staging/
    <operation-id>/
  locks/
    writer.lock
    maintenance.lock
  logs/
```

Chunk files are sharded by the first two hexadecimal characters of their hash to avoid oversized directories.

## 3. Repository Configuration

`config.toml` shall contain:

```toml
repository_id = "repo_<id>"
format_version = 0
created_at = "2026-06-02T00:00:00Z"
hash_algorithm = "blake3"
chunking = "fixed"
chunk_size_bytes = 4194304
compression = "zstd"
compression_level = 3
encrypted = false
```

Unknown fields shall be preserved where practical and ignored only when they do not affect safe reading.

## 4. Chunk Format

Each chunk file shall contain:

```text
magic             8 bytes   "TBCHUNK\0"
format_version    u16 LE     0
flags             u16 LE     reserved; must be 0
raw_size          u64 LE
stored_size       u64 LE
payload           bytes      zstd-compressed content
```

The filename is the lowercase BLAKE3 hash of the uncompressed content.

On read:

1. Validate the header and supported version.
2. Read exactly `stored_size` payload bytes.
3. Decompress the payload.
4. Verify the uncompressed length matches `raw_size`.
5. Verify the BLAKE3 hash matches the filename before restore succeeds.

Encryption fields are reserved for a later format version. Encryption design shall be finalized before the stable `1.0` repository format.

## 5. Snapshot Manifest

Published manifests shall be stored as `snapshots/snap_<id>.json`.

Minimum schema:

```json
{
  "manifest_version": 0,
  "snapshot_id": "snap_<id>",
  "state": "complete",
  "created_at": "2026-06-02T00:00:00Z",
  "sources": ["<portable-source-path>"],
  "files": [
    {
      "path": "<portable-relative-path>",
      "type": "file",
      "size": 123,
      "modified_at": "2026-06-02T00:00:00Z",
      "content_hash": "<blake3>",
      "chunks": ["<blake3>"]
    }
  ],
  "summary": {
    "file_count": 1,
    "logical_bytes": 123,
    "newly_stored_bytes": 100
  }
}
```

Supported entry types for `0.1`:

- `directory`
- `file`
- `symlink`

Unsupported special files shall be skipped with warnings. Hard-link and sparse-file metadata may be added compatibly in `0.2`.

## 6. Portable Paths and Restore Safety

- Manifest entry paths shall be relative paths using `/` separators.
- Paths shall not be empty, absolute, drive-qualified, or UNC paths.
- Paths shall not contain `.` or `..` segments.
- Restore shall join each validated path beneath the selected target and confirm that the resulting output path remains contained within that target.
- Symlinks shall be restored as symlinks where supported. Restore shall not follow a restored symlink when creating later entries.
- Platform-specific collisions, including case-insensitive collisions and reserved filenames, shall fail clearly rather than silently overwrite content.

Manifests shall be treated as untrusted input.

## 7. Backup Transaction

Only one backup writer may publish changes at a time.

1. Acquire `locks/writer.lock`.
2. Create `staging/<operation-id>/`.
3. Scan source paths. Reject a repository located inside a source tree.
4. For each regular file, read metadata before and after content streaming.
5. If a file changed while being read, retry once. If it changes again, skip it with a prominent warning. Strict mode shall fail the backup instead.
6. Write missing chunks to temporary files, validate them, then atomically rename them into `chunks/`.
7. Write the manifest to `staging/<operation-id>/snap_<id>.json`.
8. Validate the manifest and all referenced chunks.
9. Atomically rename the manifest into `snapshots/`. This is the publication point.
10. Remove the staging directory and release the writer lock.

A failed or interrupted backup may leave unreferenced chunks or staging files. It shall never expose a partial snapshot as complete.

## 8. Locks

The `0.1` implementation shall use exclusive lock files containing:

```json
{
  "operation_id": "<id>",
  "pid": 1234,
  "hostname": "<host>",
  "started_at": "2026-06-02T00:00:00Z"
}
```

- Backup publication requires `writer.lock`.
- Garbage collection later requires `maintenance.lock` and shall not run while a writer lock exists.
- A lock shall not be removed automatically merely because its process ID is absent. The CLI shall report the lock details and require an explicit recovery action.
- Network-filesystem stale-lock behavior remains an open design question.

## 9. Check and Recovery

`traceback check` shall:

- validate `config.toml`
- validate every published manifest
- verify referenced chunks exist
- verify chunk headers, decompression, sizes, and BLAKE3 hashes
- report staging directories and unreferenced chunks
- report lock files and their operation metadata

For `0.1`, abandoned data shall be reported but not automatically deleted.

## 10. Derived Index

`indexes/chunks.json` is an optional performance cache mapping chunk hashes to stored metadata. It shall never be the sole source of truth.

If the index is absent or invalid, TraceBack shall rebuild it from `chunks/` and published manifests.

## 11. Storage Accounting

- **Logical bytes:** reconstructed content size.
- **Newly stored bytes:** chunk-file bytes first added by the snapshot.
- **Unique reclaimable bytes:** bytes that become unreferenced when evaluated retained items are removed.
- **Shared bytes:** bytes referenced by multiple evaluated items.

Reports shall not count shared bytes as exclusively owned by multiple items.

## 12. Deferred Decisions

The following are intentionally deferred beyond `0.1`:

- authenticated-encryption envelope and key rotation
- metadata encryption
- content-defined chunking
- remote-backend publication protocol
- automatic stale-lock recovery on network filesystems
- hard-link preservation
- sparse-file optimization

