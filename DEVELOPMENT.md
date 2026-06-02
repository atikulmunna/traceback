# TraceBack Development Workflow

## Principles

- Prefer the simplest implementation that satisfies the current requirements.
- Avoid speculative abstractions and features outside the active milestone.
- Complete and test one feature or module before starting the next.
- Keep commits small and focused on completed, verified increments.
- Push completed commits when a Git remote is configured.

## Increment Checklist

For each feature or module:

1. Implement the smallest complete behavior.
2. Add or update focused tests.
3. Run the relevant test suite.
4. Run broader regression tests when shared behavior changes.
5. Commit the verified increment with a clear message.
6. Push the commit when a remote is available.

## Initial Build Order

1. Rust workspace and CLI skeleton.
2. Repository initialization and configuration validation.
3. Chunk encoding, decoding, hashing, and compression.
4. Snapshot manifest serialization and validation.
5. Scanner with ignore and changing-file behavior.
6. Backup transaction, locking, staging, and publication.
7. Repository check.
8. Full restore with path containment.

