# TraceBack

TraceBack is an explainable backup and restore tool written in Rust.

It aims to create deduplicated snapshots while making backup behavior easy to inspect: what changed, why storage grew, which files consume space, whether restore works, and how healthy a repository is.

## Planned Capabilities

- Local snapshot backup and restore
- BLAKE3-addressed chunk deduplication
- Integrity checks and restore rehearsal
- Snapshot diffs and storage-blame reports
- Smart ignore suggestions
- Repository health reports
- Later: encryption, remote storage, pruning, and a terminal UI

## Current Status

TraceBack is in the initial implementation phase. The Rust CLI scaffold currently exposes:

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
```

Repository initialization, verified local chunk storage, staged snapshot manifest publication, metadata-only filesystem scanning, streaming backup and restore, writer locking, interrupted-write recovery, snapshot listing, full and selected-path restore, timestamp restoration, Unix permission preservation, portable path collision checks, symlink-safe restore containment, restore rehearsal, repository integrity checks with persisted check/rehearsal history, repository doctor findings with a capability-aware reliability score, smart ignore suggestions with reviewed non-destructive application, rich snapshot diffs, backup explanations, repository-wide chunk reference accounting, and file/directory storage blame are implemented.

`snapshots`, `check`, `diff`, `explain`, `blame-size`, and `doctor` support machine-readable output with the global
`--json` flag:

```text
traceback --json snapshots --repo ./my-backups
traceback check --repo ./my-backups --json
traceback --json diff snap_old snap_new --repo ./my-backups
traceback --json explain latest --repo ./my-backups
traceback --json blame-size latest --repo ./my-backups
traceback --json doctor --repo ./my-backups
```

## Development Approach

The project is built in small, tested increments. The first milestone is a local vertical slice:

```text
init -> backup -> check -> restore
```
