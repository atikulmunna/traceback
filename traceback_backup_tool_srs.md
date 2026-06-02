# Software Requirements Specification (SRS)

# TraceBack: Explainable Backup and Restore Tool

**Version:** 1.0  
**Document Status:** Draft  
**Prepared For:** Product and Engineering Planning  
**Product Type:** Rust-based backup, deduplication, restore, and backup observability tool  

---

## 1. Introduction

### 1.1 Purpose

This Software Requirements Specification defines the functional, non-functional, architectural, and operational requirements for **TraceBack**, a backup and restore tool written in Rust.

TraceBack is designed to provide traditional backup capabilities such as snapshot creation, deduplication, compression, integrity checking, and restoration, while differentiating itself through **explainability, backup observability, storage accountability, restore rehearsal, and repository health analysis**.

Unlike conventional backup tools that primarily answer whether data has been backed up, TraceBack aims to answer deeper operational questions:

- What changed since the last backup?
- Why did repository storage increase?
- Which files or folders are consuming backup space?
- Can the user actually restore their data successfully?
- Is the backup repository healthy?
- What files should probably be ignored?
- What actions should the user take to improve backup reliability?

The goal is to create a backup system that is not only reliable, but also understandable and inspectable.

---

### 1.2 Product Vision

TraceBack is an explainable, developer-friendly backup tool that creates deduplicated snapshots and provides clear visibility into backup behavior.

The product shall combine the reliability of modern snapshot-based backup systems with an observability-first user experience.

**Core positioning:**

> TraceBack is an explainable backup tool written in Rust. It creates deduplicated snapshots like modern backup tools, but focuses on visibility: snapshot diffs, storage blame, restore rehearsal, smart ignore suggestions, and repository health reports.

---

### 1.3 Intended Audience

This document is intended for:

- Software engineers implementing the project
- Product owners defining project scope
- Open-source contributors
- QA testers validating behavior
- DevOps users evaluating backup reliability
- Technical reviewers assessing architecture and feasibility

---

### 1.4 Scope

TraceBack shall provide a command-line interface for creating, managing, verifying, explaining, and restoring backups.

The project shall be developed in phases:

1. **Core MVP**: basic backup, deduplication, snapshot management, restore, and integrity verification.
2. **Unique MVP**: explainability-focused features such as snapshot diff, storage blame, restore rehearsal, repository health reports, and smart ignore suggestions.
3. **Advanced Phase**: TUI browser, remote repositories, encryption, policy-driven automation, scheduling, HTML reports, and repository garbage collection.

TraceBack is not intended to initially replace mature enterprise backup systems. Instead, it shall focus on a developer-friendly experience and transparent backup behavior.

---

### 1.5 Definitions and Acronyms

| Term | Definition |
|---|---|
| Repository | Storage location where TraceBack keeps backup metadata, chunks, indexes, and snapshots. |
| Snapshot | A point-in-time record of files and metadata from one or more source paths. |
| Chunk | A smaller block of file content stored independently for deduplication. |
| Deduplication | Avoiding storage of duplicate content by storing identical chunks only once. |
| Manifest | Metadata document describing files, directories, chunks, hashes, permissions, and timestamps for a snapshot. |
| Restore | Reconstructing files from stored chunks and metadata. |
| Rehearsal | A test restore used to verify that backup data can actually be restored. |
| Storage Blame | A report explaining which files, directories, or snapshots consume repository space. |
| Smart Ignore | Automatic suggestions for files or directories that should likely be excluded from backup. |
| TUI | Terminal User Interface. |
| WAL | Write-Ahead Log; optional mechanism for safe metadata updates. |

---

## 2. Overall Description

### 2.1 Product Perspective

TraceBack shall operate as a local-first backup utility. The initial implementation shall work with a local filesystem repository. Later versions may support remote repositories such as SFTP, S3-compatible object storage, WebDAV, or custom remote backends.

TraceBack shall be distributed as a single binary where possible. It shall prioritize predictable performance, data safety, simple installation, and clear command-line output.

The product shall be implemented in Rust to benefit from:

- Memory safety
- High-performance file I/O
- Safe concurrency
- Reliable binary data handling
- Strong type modeling for snapshots, chunks, indexes, and manifests
- Cross-platform CLI distribution

---

### 2.2 Product Differentiation

TraceBack shall differentiate itself from traditional backup tools through visibility and user understanding.

Traditional backup tools often provide commands such as:

- backup
- restore
- check
- prune
- snapshots

TraceBack shall provide those core capabilities, but shall additionally focus on:

- Explaining why a backup stored new data
- Showing changed files between snapshots
- Showing actual deduplicated storage cost per file or folder
- Detecting unnecessary backup targets such as build artifacts and dependency folders
- Testing restore capability before disaster occurs
- Producing a repository health score with actionable recommendations
- Providing a timeline-oriented TUI for backup inspection

---

### 2.3 User Classes and Characteristics

#### 2.3.1 Individual Developer

A developer wants to back up source code, documents, notes, dotfiles, and project files while avoiding unnecessary build artifacts such as `target/`, `node_modules/`, `.next/cache/`, and virtual environments.

Primary needs:

- Fast incremental backups
- Smart ignore suggestions
- Snapshot diffs
- Easy restore of individual files
- Clear repository growth reports

#### 2.3.2 Researcher or Student

A researcher wants to back up papers, manuscripts, datasets, LaTeX projects, references, and notes.

Primary needs:

- Reliable snapshot history
- Recovery of accidentally deleted drafts
- Clear timeline of file changes
- Integrity checking
- External drive support

#### 2.3.3 Server Administrator

A server administrator wants to back up configuration files, application folders, deployment files, and database dumps.

Primary needs:

- Scriptable CLI
- Policy-based backup configuration
- Integrity checks
- Restore rehearsal
- Remote repository support
- Alert-friendly output

#### 2.3.4 Open-Source Power User

A power user wants inspectability, advanced configuration, and confidence that the backup repository is healthy.

Primary needs:

- Detailed logs
- Repository stats
- Storage blame
- Prune policies
- TUI inspection
- Machine-readable JSON output

---

### 2.4 Operating Environment

The initial version shall support:

- Linux
- macOS
- Windows, where filesystem semantics allow

The first development target should be Linux because it provides mature filesystem behavior, symlink handling, permissions, and common backup use cases.

TraceBack shall support:

- Local disks
- External drives
- Mounted network filesystems
- Later: object storage and remote backends

---

### 2.5 Design and Implementation Constraints

- The implementation language shall be Rust.
- The system shall use content hashing for deduplication.
- The repository format shall be documented.
- Snapshot metadata shall be stored in a readable or inspectable format during early versions, such as JSON, MessagePack, CBOR, or SQLite.
- The system shall avoid destructive operations unless explicitly confirmed.
- Restore behavior shall be deterministic.
- File integrity shall be verified using cryptographic hashes.
- The CLI shall be scriptable and suitable for automation.
- The Core MVP shall use fixed-size chunking and fixed `zstd` compression. Later versions may expose configurable compression profiles and content-defined chunking without invalidating existing repositories.
- The repository format shall reserve versioned fields for encryption envelopes before the format is declared stable, even when an early repository is unencrypted.

---

### 2.6 Assumptions and Dependencies

The system assumes:

- Users have permission to read source paths.
- Users have write access to the backup repository path.
- Initial repositories are local filesystem repositories.
- Users understand that backups are only reliable when stored on separate physical media or remote storage.
- Encryption may not be available in the earliest prototype, but it shall be implemented before the stable `1.0` repository format is released.

Potential Rust crates:

| Area | Candidate Crates |
|---|---|
| CLI | `clap` |
| Logging | `tracing`, `tracing-subscriber` |
| Error handling | `thiserror`, `anyhow` |
| Hashing | `blake3` |
| Compression | `zstd` |
| Serialization | `serde`, `serde_json`, `rmp-serde`, `ciborium` |
| Filesystem traversal | `walkdir`, `ignore`, `jwalk` |
| Database/index | `sled`, `redb`, `sqlite`, `rusqlite` |
| TUI | `ratatui`, `crossterm` |
| Parallelism | `rayon`, `tokio` |
| Encryption | `age`, `chacha20poly1305`, `ring` |
| Config | `toml`, `config` |

---

### 2.7 Resolved Design Gaps and Required Technical Specifications

The following design gaps were identified during SRS review. They are acknowledged as required engineering work, not optional implementation details:

- Repository transactions, locking, crash recovery, and manifest publication order.
- Source files changing while a backup is in progress.
- Cross-platform filesystem semantics for symlinks, hard links, sparse files, special files, path casing, and Unicode filenames.
- Storage accounting rules for chunks shared across files and snapshots.
- Encryption envelope design and malicious-manifest defenses before repository format stabilization.
- Capability-aware health scoring so users are not penalized for product features that are not yet available.

Before repository format stabilization, the project shall publish a repository format specification covering:

- repository lock behavior
- staging paths and atomic publication
- chunk framing and versioning
- manifest schema and validation
- crash recovery and abandoned-data cleanup
- portable path representation and restore containment checks
- storage accounting definitions
- encryption envelope and key-derivation metadata

The initial local repository format is specified in `repository_format_v0.md`.

---

## 3. Product Goals

### 3.1 Primary Goals

TraceBack shall:

1. Create reliable snapshot-based backups.
2. Deduplicate file content across snapshots.
3. Restore full snapshots or individual files.
4. Verify repository integrity.
5. Explain backup behavior in human-readable language.
6. Show how and why storage is being consumed.
7. Test restore capability through rehearsal.
8. Help users avoid backing up unnecessary files.
9. Provide automation-friendly and human-friendly interfaces.

---

### 3.2 Non-Goals for Initial MVP

The initial MVP shall not attempt to provide:

- Full enterprise backup management
- Cloud dashboard
- Multi-user access control
- Real-time continuous backup
- Kernel-level filesystem snapshots
- Distributed repository replication
- Full GUI application
- Perfect replacement for mature tools like Restic or Borg

---

## 4. Feature Phases

## 4.1 Core MVP Features

The Core MVP establishes the backup engine and basic repository model.

### MVP-001: Repository Initialization

TraceBack shall allow the user to initialize a repository.

Example:

```bash
traceback init ./my-backups
```

The repository shall contain the required structure for storing configuration, chunks, indexes, and snapshots.

Example repository layout:

```text
my-backups/
  config.toml
  chunks/
  snapshots/
  indexes/
  locks/
  logs/
```

Requirements:

- The system shall refuse to initialize a repository in a non-empty incompatible directory unless forced.
- The system shall create a repository configuration file.
- The system shall store repository format version.
- The system shall validate repository structure before use.

Acceptance criteria:

- Running `traceback init <path>` creates a usable repository.
- Running `traceback init` again on the same repository warns the user.
- Invalid repository paths produce clear errors.

---

### MVP-002: File Scanning

TraceBack shall recursively scan source paths and build a file inventory.

Requirements:

- The scanner shall detect regular files, directories, symlinks, and unsupported file types.
- The scanner shall record file size, modified time, permissions, and path.
- The scanner shall respect ignore rules when configured.
- The scanner shall avoid infinite loops caused by symlinks.
- The scanner shall report unreadable files.
- The scanner shall store symlinks as symlinks by default and shall not follow them unless explicitly configured.
- The scanner shall preserve hard-link relationships where supported, or report that files will be restored independently.
- The scanner shall identify sparse files where supported and restore them efficiently where practical.
- The scanner shall skip unsupported special files such as devices, sockets, and named pipes with a clear warning unless a future feature explicitly supports them.
- The scanner shall reject or explicitly exclude a repository path located inside a source tree.
- The scanner shall compare file metadata before and after reading content. If a file changes during backup, the system shall retry, skip with a warning, or fail according to a documented strictness policy.

Acceptance criteria:

- The scanner lists all readable files under a source path.
- Symlink loops do not crash the system.
- Permission errors are reported without aborting the entire backup unless configured as fatal.
- A repository located inside the source tree is not recursively backed up.
- Files modified while being read are not silently accepted as stable snapshot content.

---

### MVP-003: Chunking

TraceBack shall split files into chunks for deduplication.

Initial MVP may use fixed-size chunks for simplicity. Later versions may implement content-defined chunking.

Requirements:

- The system shall split files into chunks.
- The system shall compute a cryptographic hash for each chunk.
- The system shall detect whether a chunk already exists in the repository.
- The system shall only store chunks that are not already present.

Acceptance criteria:

- Repeated backup of unchanged files stores no duplicate content.
- Modified files store only new or changed chunks where possible.
- Chunk metadata is recorded in the snapshot manifest.

---

### MVP-004: Backup Command

TraceBack shall provide a backup command.

Example:

```bash
traceback backup ~/Documents --repo ./my-backups
```

Requirements:

- The command shall scan input paths.
- The command shall chunk files.
- The command shall store new chunks.
- The command shall create a snapshot manifest.
- The command shall print summary statistics.
- The command shall acquire an exclusive repository writer lock before publishing repository changes.
- The command shall write new chunks and snapshot metadata to staging locations where practical.
- The command shall publish the completed snapshot manifest last so incomplete snapshots never appear as valid snapshots.
- The command shall leave already-written content-addressed chunks recoverable or safely collectible after interruption.

Example output:

```text
Backup completed.

Files scanned:        2,341
Logical size:         2.2 GB
New data stored:      420 MB
Compressed size:      180 MB
Deduplicated:         1.8 GB
Snapshot ID:          snap_2026_06_01_001
```

Acceptance criteria:

- A backup command creates a restorable snapshot.
- Running backup twice on unchanged data stores little or no new content.
- Backup failure does not leave repository metadata in an inconsistent state.
- Interrupted backups do not publish partial snapshots.

---

### MVP-005: Snapshot Listing

TraceBack shall list available snapshots.

Example:

```bash
traceback snapshots --repo ./my-backups
```

Output:

```text
ID                    Date                  Sources
snap_001              2026-06-01 10:30      ~/Documents
snap_002              2026-06-02 11:15      ~/Documents
snap_003              2026-06-05 22:10      ~/Projects
```

Requirements:

- The system shall list snapshot ID, creation time, source paths, logical size, and stored size where available.
- The command shall support machine-readable JSON output.

Acceptance criteria:

- Users can identify available snapshots.
- JSON output can be consumed by scripts.

---

### MVP-006: Restore Command

TraceBack shall restore a snapshot or a selected file from a snapshot.

Full restore:

```bash
traceback restore snap_001 --repo ./my-backups --target ./restored
```

Single file restore:

```bash
traceback restore snap_001:/Thesis/chapter-3.docx --repo ./my-backups --target ./chapter-3.docx
```

Requirements:

- The system shall reconstruct files from stored chunks.
- The system shall restore directory structure.
- The system shall verify chunk hashes during restore.
- The system shall avoid overwriting existing files unless explicitly allowed.
- The system shall restore metadata where supported.
- The system shall validate every restored path and symlink target according to the restore policy.
- The system shall reject manifest entries that escape the selected restore target through absolute paths, parent traversal, path normalization, or symlink traversal.

Acceptance criteria:

- Restored files match original file content hashes.
- Restore fails safely if a required chunk is missing.
- Existing files are not overwritten without confirmation or a force flag.
- Malicious or malformed manifest paths cannot write outside the restore target.

---

### MVP-007: Repository Check

TraceBack shall verify repository integrity.

Example:

```bash
traceback check --repo ./my-backups
```

Requirements:

- The system shall verify that snapshot manifests are readable.
- The system shall verify that referenced chunks exist.
- The system shall verify chunk hashes.
- The system shall report missing, corrupted, or orphaned chunks.
- The system shall report abandoned staging data and interrupted transaction artifacts.

Acceptance criteria:

- Corrupted chunks are detected.
- Missing chunks are reported with affected snapshots/files.
- Successful checks produce a clear pass result.

---

### MVP-008: Basic Ignore Rules

TraceBack shall support ignore rules.

Example `.tracebackignore`:

```text
node_modules/
target/
.cache/
*.tmp
.DS_Store
```

Requirements:

- The scanner shall read ignore rules from a config file.
- The scanner shall support basic glob patterns.
- The scanner shall show how many files were ignored.

Acceptance criteria:

- Ignored files are not backed up.
- Ignore behavior is predictable and testable.

---

## 4.2 Unique MVP Features

The Unique MVP differentiates TraceBack from ordinary backup tools by making backup behavior explainable.

---

### UMVP-001: Explainable Backup Report

TraceBack shall generate a human-readable explanation after each backup.

Example:

```bash
traceback explain latest --repo ./my-backups
```

Example output:

```text
Backup Explanation: snap_2026_06_01_001

Scanned:
  18,420 files
  7.4 GB logical size

Stored:
  312 MB new chunks
  94 MB compressed size

Why new data increased:
  180 MB: new PDF files in /Research
   72 MB: modified video cache files
   31 MB: changed project build artifacts
   18 MB: renamed files, deduplicated successfully
   11 MB: metadata changes only
```

Requirements:

- The system shall categorize new stored data by directory.
- The system shall categorize new stored data by file extension where useful.
- The system shall identify large contributors to repository growth.
- The system shall distinguish logical file size from actual new stored data.
- The system shall show compression impact.

Acceptance criteria:

- Users can understand why a backup stored new data.
- Large new files are visible in the report.
- Reports are available after backup and through a later command.

---

### UMVP-002: Snapshot Diff

TraceBack shall compare two snapshots like a version-control diff.

Example:

```bash
traceback diff snap_001 snap_002 --repo ./my-backups
```

Example output:

```text
Added:
  + Research/new-paper.pdf             12.4 MB logical, 9.1 MB stored

Modified:
  ~ thesis/chapter-3.docx              +420 KB new stored data
  ~ src/main.rs                        +3 KB new stored data

Renamed:
  > old-notes/rust.md -> notes/rust.md  0 B new stored data

Deleted:
  - temp/demo.mov                      recoverable from snap_001
```

Requirements:

- The system shall detect added files.
- The system shall detect deleted files.
- The system shall detect modified files.
- The system should detect probable renames by matching file content hashes.
- The system shall show actual new storage cost where available.

Acceptance criteria:

- Users can compare two snapshots clearly.
- Renamed files do not appear only as delete/add when content is unchanged.
- JSON diff output is available for scripts.

---

### UMVP-003: Storage Blame

TraceBack shall explain repository storage usage by file, directory, extension, and snapshot.

Example:

```bash
traceback blame-size latest --repo ./my-backups
```

Example output:

```text
What is consuming repository space?

1. Videos/                      42.1 GB
2. VM Images/                   18.7 GB
3. Research PDFs/                6.4 GB
4. node_modules/                 2.8 GB
5. Pictures/                     2.1 GB

Largest new data since previous backup:

1. Videos/demo-recording.mp4     +1.4 GB
2. thesis/figures.zip            +220 MB
3. Projects/app/.next/cache      +180 MB
```

Requirements:

- The system shall calculate storage contribution by directory.
- The system shall calculate storage contribution by file extension.
- The system shall show largest files by logical size.
- The system shall show largest files by unique stored data.
- The system shall identify growth since previous snapshot.
- Reports shall distinguish logical bytes, newly stored bytes for a snapshot, and currently unique bytes that would become unreferenced if an item were removed.
- Shared chunk bytes shall not be double-counted as exclusive storage. Reports shall show shared bytes separately or attribute them proportionally using a documented deterministic rule.
- Human-readable and JSON reports shall name the accounting method used.

Acceptance criteria:

- Users can identify why repository size is growing.
- Reports distinguish logical size from deduplicated stored size.
- Aggregated storage reports reconcile with repository-level totals according to the documented accounting method.

---

### UMVP-004: Restore Rehearsal

TraceBack shall support a restore rehearsal command that validates restore readiness before disaster occurs.

Example:

```bash
traceback rehearse latest --repo ./my-backups --target /tmp/restore-test
```

Requirements:

- The system shall restore either a sample or full snapshot to a temporary or user-selected path.
- The system shall verify restored file hashes.
- The system shall verify required chunks are readable.
- The system shall optionally sample large snapshots instead of restoring everything.
- The system shall produce a pass/fail report.
- Sampled rehearsals shall clearly report their coverage and shall not claim the same confidence level as a full rehearsal.

Example output:

```text
Restore rehearsal passed.

Verified:
  2,000 sampled files
  31,842 chunks
  permissions
  symlinks
  modified times

Estimated full restore time:
  11 min 40 sec
```

Acceptance criteria:

- Rehearsal detects missing or corrupted chunks.
- Rehearsal provides confidence that restore will work.
- Rehearsal does not overwrite existing user files.
- Reports distinguish sampled rehearsal results from full restore verification.

---

### UMVP-005: Repository Doctor and Health Score

TraceBack shall provide a repository health report.

Example:

```bash
traceback doctor --repo ./my-backups
```

Example output:

```text
Backup Health: 87/100

Good:
  ✓ Last backup: 3 hours ago
  ✓ Repository check passed
  ✓ 3 restore rehearsals passed

Warnings:
  ! No offsite copy configured
  ! 41 GB of unpruned old snapshots
  ! Large cache folder is being backed up

Suggestions:
  Add ignore rule: ~/.cache
  Configure remote mirror
  Run: traceback prune --keep-daily 7 --keep-weekly 4
```

Requirements:

- The system shall evaluate repository integrity status.
- The system shall evaluate age of latest backup.
- The system shall evaluate whether restore rehearsal has recently passed.
- The system shall evaluate repository growth.
- The system shall identify risky patterns such as no offsite storage, no encryption, or excessive ignored suggestions.
- The system shall calculate a numeric health score.

Acceptance criteria:

- Health score is deterministic based on documented rules.
- Warnings include actionable recommendations.
- The command works even for small repositories.

---

### UMVP-006: Smart Ignore Suggestions

TraceBack shall suggest files and directories that should likely be ignored.

Example:

```bash
traceback ignore suggest ~/Projects --repo ./my-backups
```

Example output:

```text
Suggested ignore rules:

node_modules/                  1.2 GB
.target/                       850 MB
.cache/                        2.1 GB
.next/cache/                   340 MB
*.tmp                          80 MB
```

Requirements:

- The system shall detect common dependency folders, build outputs, caches, temporary files, and generated artifacts.
- The system shall estimate the amount of logical data that would be excluded.
- The system shall support applying suggestions interactively or through a flag.

Example:

```bash
traceback ignore apply --suggested
```

Acceptance criteria:

- The system suggests useful ignore rules for common development projects.
- Suggestions do not automatically modify configuration unless explicitly requested.
- Users can review suggestions before applying.

---

### UMVP-007: Machine-Readable Output

TraceBack shall support structured output for automation.

Example:

```bash
traceback doctor --json
traceback diff snap_001 snap_002 --json
traceback explain latest --json
```

Requirements:

- Key commands shall support JSON output.
- JSON output shall use stable field names.
- JSON output shall include error codes where applicable.

Acceptance criteria:

- Scripts can parse command output reliably.
- Human output and JSON output are both supported.

---

## 4.3 Advanced Features

Advanced features shall be implemented after the Core MVP and Unique MVP are stable.

---

### ADV-001: Terminal Timeline Browser

TraceBack shall provide an interactive terminal user interface.

Example:

```bash
traceback tui --repo ./my-backups
```

TUI views:

- Snapshot timeline
- Repository size over time
- Files changed per backup
- Largest files and directories
- Restore browser
- Ignored files
- Health warnings
- Rehearsal history

Requirements:

- The TUI shall allow browsing snapshots.
- The TUI shall allow viewing snapshot details.
- The TUI shall allow browsing files inside a snapshot.
- The TUI should allow launching restore commands safely.
- The TUI shall never perform destructive operations without confirmation.

Acceptance criteria:

- Users can inspect backup history without memorizing commands.
- TUI remains responsive for medium-sized repositories.

---

### ADV-002: Policy-Based Configuration

TraceBack shall support a policy configuration file.

Example `traceback.toml`:

```toml
[backup]
sources = ["~/Documents", "~/Projects"]
repository = "/mnt/backup/traceback"

[retention]
daily = 7
weekly = 4
monthly = 6

[ignore]
auto_detect = true
patterns = [
  "node_modules/",
  "target/",
  ".next/cache/",
  "*.tmp"
]

[verification]
rehearse_every = "7d"
sample_restore = "2GB"

[alerts]
warn_if_no_backup_for = "3d"
warn_if_no_offsite = true
```

Requirements:

- The system shall read backup policies from TOML.
- The system shall validate configuration before execution.
- The system shall support environment variable expansion where safe.
- The system shall provide clear configuration error messages.

Acceptance criteria:

- `traceback run` executes backup based on policy.
- Invalid configuration does not partially execute backup.

---

### ADV-003: Scheduled Backups

TraceBack shall support scheduled backup workflows.

Possible approaches:

- Generate systemd timer files on Linux
- Generate launchd plist files on macOS
- Generate Task Scheduler configuration on Windows
- Provide a long-running watch/scheduler daemon in later versions

Requirements:

- Users shall be able to configure scheduled backups.
- The scheduler shall use policy configuration.
- The scheduler shall record logs.
- The scheduler shall avoid running overlapping backups.

Acceptance criteria:

- Users can schedule recurring backups.
- Failed scheduled backups are reported in logs.

---

### ADV-004: Remote Repository Support

TraceBack shall support remote repositories in later versions.

Potential backends:

- SFTP
- S3-compatible object storage
- WebDAV
- Rclone-compatible backend through external integration
- Custom HTTP backend

Requirements:

- The repository interface shall abstract local and remote storage operations.
- Remote repositories shall support chunk upload, chunk download, manifest upload, and index sync.
- Remote backends shall handle transient network failures.
- Remote operations shall support retry behavior.

Acceptance criteria:

- Users can back up to at least one remote backend.
- Interrupted remote backups do not corrupt repository state.

---

### ADV-005: Encryption

TraceBack shall support encryption for stored chunks and metadata.

Requirements:

- The system shall encrypt chunk content before writing to repository.
- The system shall encrypt sensitive metadata, including stored paths, before production use.
- The system shall require a passphrase or key file.
- The system shall never store raw encryption passphrases.
- The system shall provide clear recovery warnings.
- The system shall use an audited authenticated-encryption construction and a secure KDF with versioned parameters.
- The repository format shall support encryption envelope versioning and future key rotation.

Acceptance criteria:

- Encrypted repositories cannot be read without credentials.
- Restore works correctly after unlocking repository.
- Incorrect passphrases fail safely.

---

### ADV-006: Compression Profiles

TraceBack shall support configurable compression. Core MVP repositories use fixed `zstd` compression; this feature exposes profile selection and tuning.

Example:

```bash
traceback backup ~/Documents --compression zstd:3
```

Requirements:

- The system shall support at least no compression and zstd compression.
- The system shall allow compression level configuration.
- The system shall report compression ratio.

Acceptance criteria:

- Compression behavior is configurable.
- Restore correctly decompresses chunks.

---

### ADV-007: Content-Defined Chunking

TraceBack shall support content-defined chunking to improve deduplication when file content shifts.

Requirements:

- The system shall implement or use a rolling hash chunking algorithm.
- The system shall support average chunk size configuration.
- The system shall preserve backward compatibility with fixed-size chunk repositories where possible.

Acceptance criteria:

- Insertions near the beginning of large files do not cause entire files to be re-stored.
- Deduplication improves for modified large files.

---

### ADV-008: Garbage Collection and Pruning

TraceBack shall support removing unneeded chunks after snapshots are deleted or retention policies are applied.

Example:

```bash
traceback prune --keep-daily 7 --keep-weekly 4 --keep-monthly 6
traceback gc
```

Requirements:

- The system shall identify chunks no longer referenced by any retained snapshot.
- The system shall delete unreferenced chunks safely.
- The system shall support dry-run mode.
- The system shall protect recent snapshots by default.
- Garbage collection shall acquire a repository maintenance lock that prevents races with backup, restore publication, and other garbage collection operations.
- Garbage collection shall determine liveness from published manifests only and shall handle abandoned staging data conservatively.

Acceptance criteria:

- Pruning reduces repository size when old snapshots are removed.
- Referenced chunks are never deleted.
- Dry-run accurately reports expected changes.

---

### ADV-009: HTML Backup Reports

TraceBack shall generate static HTML reports.

Example:

```bash
traceback report latest --format html --output report.html
```

Report sections:

- Backup summary
- Snapshot diff
- Storage blame
- Health score
- Smart ignore suggestions
- Restore rehearsal status
- Repository size trend

Requirements:

- Reports shall be self-contained where practical.
- Reports shall not expose secrets.
- Reports shall be suitable for sharing with technical users.

Acceptance criteria:

- HTML report opens locally in a browser.
- Report accurately reflects backup metadata.

---

### ADV-010: Watch Mode

TraceBack shall support watch mode for detecting file changes and triggering backup actions.

Example:

```bash
traceback watch --config traceback.toml
```

Requirements:

- The system shall watch configured paths.
- The system shall debounce frequent changes.
- The system shall avoid backing up temporary partial writes where possible.
- The system shall log triggered backups.

Acceptance criteria:

- Watch mode detects file changes.
- Frequent changes do not create excessive snapshots.

---

## 5. System Requirements

## 5.1 Functional Requirements

### FR-001: Repository Management

The system shall create, validate, open, inspect, lock, and recover repositories.

Priority: High  
Phase: Core MVP

---

### FR-002: Snapshot Creation

The system shall create snapshots from one or more source paths using staged writes and atomic manifest publication.

Priority: High  
Phase: Core MVP

---

### FR-003: Deduplicated Chunk Storage

The system shall store file content as deduplicated chunks addressed by cryptographic hash.

Priority: High  
Phase: Core MVP

---

### FR-004: Snapshot Manifest Storage

The system shall store a manifest for every snapshot containing file metadata and chunk references.

Priority: High  
Phase: Core MVP

---

### FR-005: Full Snapshot Restore

The system shall restore all files from a selected snapshot while enforcing restore-target containment.

Priority: High  
Phase: Core MVP

---

### FR-006: Partial Restore

The system shall restore a specific file or directory from a selected snapshot.

Priority: High  
Phase: Core MVP

---

### FR-007: Integrity Check

The system shall verify chunk existence, chunk hash validity, manifest consistency, and abandoned transaction artifacts.

Priority: High  
Phase: Core MVP

---

### FR-008: Snapshot Listing

The system shall list snapshots with metadata.

Priority: High  
Phase: Core MVP

---

### FR-009: Ignore Rule Support

The system shall exclude files based on ignore rules.

Priority: Medium  
Phase: Core MVP

---

### FR-010: Snapshot Diff

The system shall compare two snapshots and report added, modified, deleted, and renamed files.

Priority: High  
Phase: Unique MVP

---

### FR-011: Explainable Backup Report

The system shall explain what caused new data to be stored in a backup.

Priority: High  
Phase: Unique MVP

---

### FR-012: Storage Blame

The system shall report logical, newly stored, unique reclaimable, and shared bytes for files, folders, extensions, and snapshots.

Priority: High  
Phase: Unique MVP

---

### FR-013: Restore Rehearsal

The system shall test restore capability without requiring an actual disaster recovery event.

Priority: High  
Phase: Unique MVP

---

### FR-014: Repository Doctor

The system shall generate a health score and actionable recommendations.

Priority: High  
Phase: Unique MVP

---

### FR-015: Smart Ignore Suggestions

The system shall recommend files and directories that should likely be excluded from backup.

Priority: Medium  
Phase: Unique MVP

---

### FR-016: JSON Output

The system shall support structured JSON output for key commands.

Priority: Medium  
Phase: Unique MVP

---

### FR-017: Policy Configuration

The system shall support a TOML configuration file for backup policy.

Priority: Medium  
Phase: Advanced

---

### FR-018: TUI Browser

The system shall provide an interactive terminal interface for exploring backups.

Priority: Medium  
Phase: Advanced

---

### FR-019: Remote Repository

The system shall support at least one remote repository backend.

Priority: Medium  
Phase: Advanced

---

### FR-020: Encryption

The system shall encrypt repository content when configured.

Priority: High for real-world use  
Phase: Advanced or early if security is prioritized

---

## 5.2 Non-Functional Requirements

### NFR-001: Performance

TraceBack shall handle large directory trees efficiently.

Targets:

- Scan 100,000 files without excessive memory growth.
- Process large files in streaming mode.
- Avoid loading entire large files into memory.
- Use parallelism for hashing and compression where safe.

---

### NFR-002: Reliability

TraceBack shall prioritize data integrity.

Requirements:

- Snapshot publication shall be atomic: a manifest is visible as completed only after its referenced metadata is durably written.
- Incomplete snapshots shall not appear as valid completed snapshots.
- Chunk hashes shall be verified before restore.
- Repository corruption shall be detectable.
- Repository writers shall use locks with documented stale-lock recovery behavior.
- Repository maintenance operations shall not race with active snapshot publication.
- Crash recovery shall detect abandoned staging data and preserve published snapshots.

---

### NFR-003: Safety

TraceBack shall avoid destructive behavior by default.

Requirements:

- Restore shall not overwrite files unless explicitly requested.
- Prune and garbage collection shall support dry-run mode.
- Delete operations shall require confirmation unless non-interactive force flags are used.

---

### NFR-004: Usability

TraceBack shall provide clear, helpful CLI output.

Requirements:

- Error messages shall include cause and suggested fix where possible.
- Commands shall include examples in help text.
- Progress output shall be concise but informative.

---

### NFR-005: Scriptability

TraceBack shall be usable in scripts and automation.

Requirements:

- Commands shall use meaningful exit codes.
- JSON output shall be available for key commands.
- Non-interactive mode shall be supported.

---

### NFR-006: Cross-Platform Behavior

TraceBack should work across Linux, macOS, and Windows.

Requirements:

- Platform-specific metadata shall be handled gracefully.
- Unsupported metadata shall not prevent content restore.
- Path handling shall be platform-aware.
- Portable manifest paths shall use a documented normalized representation.
- Case collisions, Unicode normalization differences, reserved filenames, and long-path limitations shall produce deterministic restore behavior and clear warnings.

---

### NFR-007: Security

TraceBack shall protect backup data when encryption is enabled.

Requirements:

- Secrets shall not be logged.
- Encryption keys shall not be stored in plaintext.
- Passphrases shall not appear in command histories where avoidable.
- Restore shall treat manifests as untrusted input and prevent writes outside the selected restore target.
- Repository metadata shall be validated before use.

---

### NFR-008: Maintainability

The codebase shall be modular.

Suggested modules:

- CLI
- Repository
- Scanner
- Chunker
- Hasher
- Compressor
- Snapshot manifest
- Restore engine
- Diff engine
- Explain engine
- Doctor engine
- Ignore engine
- Storage backend abstraction

---

## 6. CLI Specification

## 6.1 Command Overview

| Command | Phase | Description |
|---|---|---|
| `traceback init <repo>` | Core MVP | Initialize repository. |
| `traceback backup <paths>` | Core MVP | Create snapshot from source paths. |
| `traceback snapshots` | Core MVP | List snapshots. |
| `traceback restore <snapshot>` | Core MVP | Restore snapshot or path. |
| `traceback check` | Core MVP | Verify repository integrity. |
| `traceback diff <a> <b>` | Unique MVP | Compare snapshots. |
| `traceback explain <snapshot>` | Unique MVP | Explain backup behavior. |
| `traceback blame-size <snapshot>` | Unique MVP | Show repository space attribution. |
| `traceback rehearse <snapshot>` | Unique MVP | Test restore capability. |
| `traceback doctor` | Unique MVP | Show backup health score. |
| `traceback ignore suggest` | Unique MVP | Suggest ignore rules. |
| `traceback ignore apply` | Unique MVP | Apply selected ignore suggestions. |
| `traceback tui` | Advanced | Open TUI browser. |
| `traceback run` | Advanced | Run configured backup policy. |
| `traceback prune` | Advanced | Apply retention policy. |
| `traceback gc` | Advanced | Remove unreferenced chunks. |
| `traceback report` | Advanced | Generate HTML/JSON reports. |
| `traceback watch` | Advanced | Watch files and trigger backups. |

---

## 6.2 Global Flags

| Flag | Description |
|---|---|
| `--repo <path>` | Repository path. |
| `--config <path>` | Configuration file path. |
| `--json` | Output JSON. |
| `--quiet` | Reduce output. |
| `--verbose` | Increase output detail. |
| `--no-progress` | Disable progress bars for CI logs. |
| `--force` | Allow potentially destructive operation. |
| `--dry-run` | Show planned changes without applying them. |

---

## 7. Data Model

## 7.1 Repository Configuration

Fields:

- repository ID
- repository format version
- creation timestamp
- chunking algorithm
- hash algorithm
- compression settings
- encryption status
- metadata format version

Example:

```toml
repository_id = "repo_01HXABC123"
format_version = 1
created_at = "2026-06-01T10:30:00Z"
hash_algorithm = "blake3"
chunking = "fixed"
chunk_size = "4MiB"
compression = "zstd:3"
encrypted = false
```

---

## 7.2 Snapshot Manifest

Each snapshot manifest shall contain:

- snapshot ID
- parent snapshot ID, if applicable
- creation timestamp
- snapshot state or format marker indicating a fully published manifest
- source paths
- hostname
- username, optional
- TraceBack version
- list of file entries
- summary statistics

File entry fields:

- path
- file type
- size
- modified time
- permissions
- content hash
- chunk references
- symlink target, if applicable
- hard-link identity, if applicable
- sparse-file metadata, if supported

Manifest paths shall use a portable normalized representation. Restore shall validate manifest paths before creating files or directories.

---

## 7.3 Chunk Metadata

Chunk metadata shall contain:

- chunk hash
- compressed size
- uncompressed size
- compression algorithm
- encryption metadata, if applicable
- reference count or index references, if maintained

---

## 7.4 Backup Report Metadata

For explainability, TraceBack should store report metadata per snapshot:

- new chunks stored
- reused chunks
- changed files
- added files
- deleted files compared with previous snapshot
- largest contributors to new storage
- ignored files summary
- compression ratio

---

## 7.5 Repository Transactions and Storage Accounting

Repository mutation shall follow a documented transaction protocol:

1. Acquire the required repository lock.
2. Write new chunks using content-addressed names. Existing valid chunks may be reused.
3. Write snapshot metadata to a staging location.
4. Validate required chunk references and manifest structure.
5. Durably publish the completed snapshot manifest as the final visibility step.
6. Release the repository lock.

Interrupted operations may leave unreferenced chunks or staging artifacts, but shall not expose an incomplete snapshot as valid. The `check` command shall report abandoned artifacts, and garbage collection shall remove them only when they are not referenced by any published manifest.

Storage reports shall use the following terms:

- **Logical bytes**: total reconstructed file content size.
- **Newly stored bytes**: physical chunk bytes first added by a specific snapshot after compression and encryption overhead where applicable.
- **Unique reclaimable bytes**: physical bytes that would become unreferenced if the selected snapshot, file, or directory were removed from the evaluated retained set.
- **Shared bytes**: physical bytes referenced by more than one evaluated item.

Reports shall avoid double-counting shared bytes and shall expose their attribution method.

---

## 8. Workflows

## 8.1 First-Time Backup Workflow

1. User initializes repository.
2. User runs backup command.
3. System acquires the repository writer lock.
4. System scans source files and applies ignore rules.
5. System chunks, hashes, and compresses files while detecting files that change during reading.
6. System writes new chunks and snapshot metadata to staging locations where practical.
7. System validates the staged snapshot.
8. System durably publishes the completed snapshot manifest as the final visibility step.
9. System releases the repository writer lock.
10. System prints backup summary.
11. System optionally prints explanation report.

---

## 8.2 Incremental Backup Workflow

1. User runs backup command again.
2. System acquires the repository writer lock.
3. System scans current file state and applies the changing-file policy.
4. System compares chunks against repository index.
5. System stores only new chunks.
6. System stages, validates, and durably publishes the completed snapshot manifest.
7. System releases the repository writer lock.
8. System generates diff against previous snapshot.
9. System explains new storage growth.

---

## 8.3 Restore Workflow

1. User selects snapshot or file path inside snapshot.
2. System validates repository.
3. System reads snapshot manifest.
4. System validates manifest paths and verifies required chunk availability.
5. System reconstructs files while enforcing restore-target containment.
6. System verifies restored content.
7. System reports success or failure.

---

## 8.4 Restore Rehearsal Workflow

1. User runs rehearsal command.
2. System selects full or sampled restore set.
3. System restores to temporary target.
4. System verifies content and metadata.
5. System deletes temporary files if configured.
6. System stores rehearsal result.
7. System updates health score inputs.

---

## 8.5 Doctor Workflow

1. User runs doctor command.
2. System checks repository integrity status.
3. System checks latest backup age.
4. System checks rehearsal history.
5. System analyzes repository growth.
6. System checks ignore suggestions.
7. System computes health score.
8. System outputs warnings and recommendations.

---

## 9. Repository Health Score Model

The health score shall be a deterministic, capability-aware reliability score from 0 to 100. A user shall not lose points because the installed TraceBack version does not yet implement a feature.

Example scoring model for a version where all listed capabilities are available:

| Category | Points |
|---|---:|
| Latest backup is recent | 20 |
| Repository check passed | 25 |
| Restore rehearsal passed recently | 20 |
| Repository has no missing/corrupt chunks | 15 |
| Ignore rules are reasonable | 5 |
| Retention/prune policy configured | 5 |
| Offsite or remote copy configured | 5 |
| Encryption enabled | 5 |

Categories unavailable in the installed product version shall be marked `not evaluated` and omitted from the denominator. Missing but available capabilities, such as an unconfigured remote copy or disabled encryption, shall reduce the score or appear as posture recommendations according to the documented scoring version.

The output shall separate:

- **Reliability score**: evidence that published backups are recent, intact, and restorable.
- **Posture recommendations**: optional hardening actions such as offsite copies, encryption, and retention policy configuration.

The scoring model and scoring-version identifier shall be documented and may evolve between major versions.

---

## 10. Error Handling Requirements

TraceBack shall provide clear error messages.

Example:

```text
Error: Cannot restore snapshot snap_002.
Reason: Required chunk b3f1...9c is missing.
Affected file: thesis/chapter-3.docx
Suggested action: Run `traceback check --repo ./my-backups` for a full report.
```

Error categories:

- repository not found
- invalid repository format
- permission denied
- unreadable source file
- missing chunk
- corrupted chunk
- invalid snapshot ID
- restore target exists
- unsupported platform metadata
- configuration error
- remote backend error

---

## 11. Logging Requirements

TraceBack shall log important events:

- repository initialization
- backup start and end
- files skipped due to errors
- chunks stored
- snapshot manifest written
- restore operations
- integrity check results
- doctor results
- prune and garbage collection operations

Logs shall avoid exposing secrets.

---

## 12. Security Requirements

### 12.1 Data Integrity

- Every chunk shall be content-addressed by hash.
- Restore shall verify chunk hashes.
- Check command shall detect corruption.
- Completed snapshot manifests shall be published atomically after referenced data is durably written.

### 12.2 Encryption

When encryption is enabled:

- Content shall be encrypted before storage.
- Metadata containing sensitive paths shall be encrypted before production use.
- Key derivation shall use a secure KDF.
- Encryption shall provide authenticity as well as confidentiality.
- Encryption envelope and KDF parameters shall be versioned.
- Wrong passphrases shall fail safely.

### 12.3 Safe Defaults

- Destructive commands shall require confirmation.
- Restore shall not overwrite files by default.
- Logs shall not print passphrases, keys, or sensitive environment values.
- Restore shall reject absolute paths, parent traversal, and other manifest entries that escape the selected restore target.

---

## 13. Performance Requirements

### 13.1 Streaming

Large files shall be processed in streams. The system shall not require loading entire files into memory.

### 13.2 Parallelism

The system should parallelize:

- file hashing
- compression
- chunk writes
- scanning where practical

Parallelism shall not compromise repository consistency.

### 13.3 Progress Reporting

Long operations shall display progress:

- files scanned
- bytes processed
- chunks stored
- estimated completion where possible

Progress output shall be disableable for CI environments.

---

## 14. Testing Requirements

## 14.1 Unit Tests

Unit tests shall cover:

- chunking
- hashing
- compression and decompression
- manifest serialization
- ignore rule matching
- diff classification
- storage blame calculations
- health score calculations
- manifest path validation and restore-target containment
- changing-file detection policy
- storage accounting reconciliation for shared chunks

## 14.2 Integration Tests

Integration tests shall cover:

- init → backup → restore workflow
- repeated backup deduplication
- corrupted chunk detection
- missing chunk detection
- restore rehearsal
- smart ignore suggestion
- prune dry-run
- interruption at each repository write and snapshot publication phase
- restart and cleanup after interrupted backup
- concurrent backup attempts and stale-lock recovery
- backup versus garbage-collection locking
- files modified, removed, or replaced while being backed up
- repository path located inside a source tree
- malicious manifests containing absolute paths, parent traversal, and symlink escape attempts

## 14.3 Property-Based Tests

Property-based tests should verify:

- restored content equals original content
- chunking and reconstruction are inverse operations
- manifests round-trip through serialization
- diff results remain consistent for generated file trees
- storage accounting totals reconcile for generated shared-chunk graphs
- normalized restore paths never escape the selected restore target

## 14.4 Cross-Platform Tests

Tests should validate:

- path handling
- symlink behavior
- permissions behavior
- Unicode filenames
- long paths where supported
- case-colliding paths
- Unicode normalization differences
- reserved filenames where applicable

---

## 15. Release Plan

## 15.1 Version 0.1: Core Backup Engine

Included:

- repository init
- file scanning
- fixed-size chunking
- BLAKE3 hashing
- fixed `zstd` compression
- local chunk storage
- backup command
- snapshot manifest
- snapshot list
- full restore
- repository check
- repository locking and atomic manifest publication
- basic changing-file detection

---

## 15.2 Version 0.2: Repository Hardening and Partial Restore

Included:

- partial restore
- basic ignore rules
- safer error handling
- crash recovery and abandoned staging-data reporting
- documented repository format specification
- cross-platform path normalization and restore containment
- shared-chunk storage accounting
- concurrency and fault-injection test coverage

---

## 15.3 Version 0.3: Explainability MVP

Included:

- snapshot diff
- explain report
- storage blame
- JSON output

---

## 15.4 Version 0.4: Reliability MVP

Included:

- restore rehearsal
- repository doctor
- health score
- smart ignore suggestions

---

## 15.5 Version 0.5: Production Hardening

Included:

- policy config
- pruning and garbage collection
- encryption
- at least one remote backend

---

## 15.6 Version 0.6: Advanced UX

Included:

- TUI timeline browser
- HTML reports
- compression profiles
- scheduled backups

---

## 15.7 Version 1.0: Stable Backup Tool

Included:

- stable repository format
- documented CLI
- robust check and restore
- explainability features
- pruning and garbage collection
- encryption
- at least one remote backend

---

## 16. Success Metrics

TraceBack shall be considered successful if:

- It can reliably back up and restore real user directories.
- Repeated backups deduplicate unchanged data.
- Users can understand repository growth through explain reports.
- Restore rehearsal detects broken backups before real restore events.
- Doctor command provides actionable recommendations.
- The CLI is usable by both humans and scripts.
- The repository format is documented enough for technical users to inspect.

---

## 17. Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Repository corruption | Very high | Atomic writes, check command, temp files, manifest validation. |
| Interrupted backup publication | Very high | Writer locks, staging, publish manifest last, crash-injection testing. |
| Source file changes during backup | High | Compare metadata before and after reading; retry, warn, or fail according to strictness policy. |
| Poor deduplication | Medium | Start with fixed chunks, later add content-defined chunking. |
| Slow scanning | Medium | Parallel traversal, ignore rules, incremental metadata cache. |
| Complex cross-platform metadata | Medium | Implement core content restore first, metadata best-effort. |
| Dangerous restore overwrite | High | No overwrite by default, require force flag. |
| Malicious manifest paths | Very high | Validate normalized paths and enforce restore-target containment. |
| Backup and garbage-collection race | Very high | Repository maintenance locks and liveness checks based on published manifests only. |
| Encryption mistakes | High | Use audited authenticated-encryption libraries; avoid custom crypto; version encryption envelopes. |
| Scope creep | High | Separate Core MVP, Unique MVP, and Advanced phases. |

---

## 18. Recommended Architecture

### 18.1 Module Layout

```text
traceback/
  crates/
    traceback-cli/
    traceback-core/
    traceback-repo/
    traceback-scan/
    traceback-chunk/
    traceback-restore/
    traceback-diff/
    traceback-explain/
    traceback-doctor/
    traceback-tui/
```

### 18.2 Core Components

#### CLI Layer

Responsible for parsing commands, flags, and configuration.

#### Repository Layer

Responsible for reading and writing repository data, locking, staging, atomic snapshot publication, and crash recovery.

#### Scanner Layer

Responsible for walking source directories and applying ignore rules.

#### Chunker Layer

Responsible for splitting files into chunks.

#### Index Layer

Responsible for determining whether chunks already exist.

#### Snapshot Layer

Responsible for manifest creation and reading.

#### Restore Layer

Responsible for reconstructing files from chunks, validating untrusted manifest paths, and enforcing restore-target containment.

#### Diff Layer

Responsible for comparing snapshots.

#### Explain Layer

Responsible for backup reports and storage growth explanations.

#### Doctor Layer

Responsible for repository health scoring and recommendations.

#### TUI Layer

Responsible for interactive terminal visualization.

---

## 19. Open Questions

Resolved for initial implementation:

- Metadata shall use readable versioned JSON manifests and a simple derived index that can be rebuilt.
- The initial repository format shall prioritize human inspectability and correctness before maximum performance.
- Source-file changes shall retry once, then skip with a prominent warning. Strict mode shall fail the backup instead.

Remaining questions:

1. Should TraceBack support Windows metadata fully or provide best-effort content backup first?
2. Should restore rehearsal sample files by count, size, risk, or random selection?
3. Should smart ignore rules be built-in or maintained as an external rules registry?
4. How should stale locks be detected safely across local disks and mounted network filesystems?
5. Which authenticated-encryption library and key-rotation strategy should be adopted?

---

## 20. Final Product Summary

TraceBack shall be a Rust-based explainable backup tool that combines deduplicated snapshot backups with visibility-first features.

The Core MVP shall prove that the system can safely back up, deduplicate, list, check, and restore data.

The Unique MVP shall make the product meaningfully different through:

- Explainable backup reports
- Snapshot diff
- Storage blame
- Restore rehearsal
- Repository doctor
- Smart ignore suggestions
- JSON automation output

The Advanced phase shall improve usability and production readiness through:

- TUI timeline browser
- Policy-based configuration
- Scheduled backups
- Remote storage
- Encryption
- Compression profiles
- Content-defined chunking
- Pruning and garbage collection
- HTML reports
- Watch mode

The central promise of TraceBack is:

> Backups should not be a black box. TraceBack helps users understand what was backed up, what changed, why storage grew, and whether restore will actually work.
