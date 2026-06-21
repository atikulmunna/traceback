# TraceBack

TraceBack is an explainable backup and restore tool written in Rust.

It aims to create deduplicated snapshots while making backup behavior easy to inspect: what changed, why storage grew, which files consume space, whether restore works, and how healthy a repository is.

## Capabilities

- Local snapshot backup and restore
- BLAKE3-addressed chunk deduplication
- Integrity checks and restore rehearsal
- Snapshot diffs and storage-blame reports
- Smart ignore suggestions
- Repository health reports
- Optional encryption, filesystem remote push, and pruning
- Terminal UI snapshot and file browser with selected-entry details

## Install

TraceBack currently builds from source:

```text
cargo install --path crates/traceback-cli
```

For local development, run the binary through Cargo:

```text
cargo run -p traceback-cli -- --help
```

## Quickstart

Create a repository, back up a source directory, verify it, and restore it:

```text
traceback init ./my-backups
traceback backup ./documents --repo ./my-backups
traceback check --repo ./my-backups
traceback snapshots --repo ./my-backups
traceback restore <snapshot-id> --repo ./my-backups --target ./restore-test
```

For encrypted repositories, provide the passphrase through an environment
variable:

```text
traceback init ./my-backups --encrypted --passphrase-env TRACEBACK_PASSPHRASE
```

Durable repository objects can be pushed to a filesystem remote:

```text
traceback remote push --repo ./my-backups --remote file:///backups/traceback
```

## Commands

The CLI currently exposes:

```text
init
backup
snapshots
restore
rehearse
check
recover
diff
explain
blame-size
doctor
ignore suggest
ignore apply
gc
prune
run
remote push
tui
```

Repository initialization, optional encrypted chunk storage, verified local chunk storage behind a storage abstraction, filesystem remote push, staged snapshot manifest publication, metadata-only filesystem scanning, streaming backup and restore, policy-based backup runs, writer locking, interrupted-write recovery, resilience coverage for stale locks and maintenance contention, snapshot listing, full and selected-path restore, timestamp restoration, Unix permission preservation, portable path collision checks, symlink-safe restore containment, restore rehearsal, repository integrity checks with persisted check/rehearsal history, repository doctor findings with a capability-aware reliability score, smart ignore suggestions with reviewed non-destructive application, rich snapshot diffs, backup explanations, repository-wide chunk reference accounting, file/directory storage blame, garbage collection, snapshot pruning, and a terminal snapshot/file browser with detail inspection are implemented.

The terminal browser validates a repository and opens a read-only snapshot
timeline with a file list for the selected snapshot. Use `Tab` to switch focus,
`Up`/`Down` or `j`/`k` to move, `Home`/`End` to jump, `/` to filter snapshot
paths, `c` to clear the file filter, `?` for help, and `q`/`Esc` to quit. The
detail panel shows selected snapshot metadata plus selected file type, size,
content hash, and chunk references:

```text
traceback tui --repo ./my-backups
```

Human-readable commands support global `--quiet`, `--verbose`, and
`--no-progress` output controls.

`snapshots`, `check`, `diff`, `explain`, `blame-size`, `doctor`, `gc`, `prune`, and `remote push` support machine-readable output with the global
`--json` flag:

```text
traceback --json snapshots --repo ./my-backups
traceback check --repo ./my-backups --json
traceback --json diff snap_old snap_new --repo ./my-backups
traceback --json explain latest --repo ./my-backups
traceback --json blame-size latest --repo ./my-backups
traceback --json doctor --repo ./my-backups
traceback --json gc --repo ./my-backups --dry-run
traceback --json prune --repo ./my-backups --keep-latest 3 --dry-run
traceback --json remote push --repo ./my-backups --remote /backups/traceback
```

Policy backups can be run from a versioned TOML file:

```text
traceback run --config traceback.toml
```

## Repository Compatibility

TraceBack writes versioned repository metadata and validates repository
configuration before mutating data. The current repository format is
`format_version = 0`, with versioned snapshot manifests and a versioned
encrypted chunk envelope when encryption is enabled.

Until `1.0`, incompatible format changes may still happen. The project policy
is to reject unsupported repository versions explicitly instead of attempting a
best-effort read. Future migrations should be implemented as deliberate,
tested commands rather than silent rewrites.

## Release Process

Every change should pass the same gate used by CI:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Tagged releases matching `v*` build Linux and Windows archives through GitHub
Actions.

## Development Approach

The project is built in small, tested increments. The first milestone is a local vertical slice:

```text
init -> backup -> check -> restore
```
