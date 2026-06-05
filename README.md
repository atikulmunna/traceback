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
check
diff
```

Repository initialization, verified local chunk storage, snapshot manifest serialization, filesystem scanning, backup creation, snapshot listing, full snapshot restore, repository integrity checks, and snapshot diffs are implemented.

## Development Approach

The project is built in small, tested increments. The first milestone is a local vertical slice:

```text
init -> backup -> check -> restore
```
