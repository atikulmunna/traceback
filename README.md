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
- Guided terminal UI with snapshot and file browser details

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

Repository initialization, optional encrypted chunk storage, verified local chunk storage behind a storage abstraction, filesystem remote push, staged snapshot manifest publication, metadata-only filesystem scanning, streaming backup and restore, policy-based backup runs, writer locking, interrupted-write recovery, resilience coverage for stale locks and maintenance contention, snapshot listing, full and selected-path restore, timestamp restoration, Unix permission preservation, portable path collision checks, symlink-safe restore containment, restore rehearsal, repository integrity checks with persisted check/rehearsal history, repository doctor findings with a capability-aware reliability score, smart ignore suggestions with reviewed non-destructive application, rich snapshot diffs, backup explanations, repository-wide chunk reference accounting, file/directory storage blame, garbage collection, snapshot pruning, and a guided terminal UI with snapshot/file detail inspection are implemented.

The terminal UI validates a repository and opens a guided main menu:

```text
traceback tui --repo ./my-backups
```

If the repository path does not exist yet, the TUI still opens and offers an
`Initialize repository` action from the main menu.

To skip typing a long backup source path inside the terminal UI, launch it with
the source prefilled:

```text
traceback tui --repo ./my-backups --source ./documents
```

The main menu lets users choose guided actions such as changing or initializing
the repository, running a backup from a selected source, browsing snapshots,
restoring files, rehearsing restores, checking repository health, reviewing the
doctor report, comparing snapshots, explaining backups, reviewing storage blame,
recovering interrupted writes, or exiting.

The snapshot browser shows three panels: snapshots, files in the selected
snapshot, and details for the selected snapshot or file. Restore support previews
the exact target and equivalent `traceback restore ...` command first. The TUI
uses a safe default restore target beside the repository under `traceback-restore`.
Users can override that target with `t`; the TUI only writes after the restore is
confirmed with `y`.

The repository health screen runs the same integrity check as `traceback check`
and shows pass/fail status, verified manifest and chunk counts, staging or
temporary leftovers, orphaned chunks, and issue guidance.

The doctor report screen runs the same reliability report as `traceback doctor`
and shows the health score, latest backup evidence, current integrity status,
recorded check/rehearsal evidence, findings, and recommendations.

The restore rehearsal screen verifies that a selected snapshot can be restored
into a temporary directory without writing to a user-selected destination.

The snapshot diff screen lets users choose old and new snapshots, run the same
comparison as `traceback diff`, and review added, removed, modified, and
unchanged path counts with a short changed-path list.

The explain backup screen runs the same analysis as `traceback explain` for a
selected snapshot and shows change counts, logical/stored bytes, new versus
reused chunk bytes, and the top growth contributors.

The storage blame screen runs the same accounting as `traceback blame-size` for
a selected snapshot and shows total logical, unique, shared, and reclaimable
stored bytes plus the top paths by stored impact.

The recover interrupted writes screen scans for abandoned staging entries and
temporary chunk files, shows the cleanup plan, and requires `y` confirmation
before running the same cleanup as `traceback recover`.

TUI keybindings:

```text
Tab             switch focus between snapshots and files
Up/Down, j/k    move within the focused panel
Home/End        jump to the first or last item in the focused panel
Enter           select a main-menu item
Enter           run restore rehearsal on the rehearsal screen
Enter           rerun the repository health check on the health screen
Enter           rerun the doctor report on the doctor screen
Enter           run the selected snapshot diff on the diff screen
Enter           explain the selected snapshot on the explain screen
Enter           run storage blame on the storage blame screen
Enter           scan for recoverable artifacts on the recovery screen
y               confirm restore or reviewed maintenance cleanup
n               cancel or clear a restore/maintenance preview
Backspace       return from the browser to the main menu
e               edit the backup source on the backup review screen
/               start filtering file paths
Enter           accept the current file filter
t               optionally enter a custom restore target path from the browser
Esc             stop filtering, cancel restore preview, or quit
c               clear the file filter while the file panel is focused
r               preview a restore for the focused snapshot/file
? or F1         show or hide help
q               quit
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
