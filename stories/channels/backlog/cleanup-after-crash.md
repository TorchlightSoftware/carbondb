# A refused op leaves no spaces behind, even when the device crashes

Status: backlog

## Why

When an op that created a channel or service is refused, the core deletes the spaces its run created, and those their own ops created in turn (`Carbon::unmake` in `crates/core/src/carbon.rs`). That cleanup runs after the refused result commits, so a crash between the two leaves the files behind: the op is settled and never runs again, so nothing deletes them.

What is left depends on where the space was recorded. A space created inside the system's own op, such as founding an identity, is recorded in the system database in that op's transaction, so a refusal rolls the record back and leaves only an unrecorded file. A space created by a service's op, such as `POST [channel]/services`, is recorded in the system database at once, outside the caller's transaction, so it stays recorded as a space this device holds, though no channel names it.

Retrying is not blocked: a new op derives new ids, so the leftovers are only wasted disk and, for a recorded one, a space opened on every start.

## Idea

Cleanup becomes something the device finishes on its next start rather than something it must finish before a crash. On open, the core finds the spaces no settled op still accounts for and deletes them, the same way whichever op created them.

## To decide

- How the core tells a leftover from a space still in use: a file with no record in the system database, a record that no channel names, or the `create` effects recorded on refused ops.
- Whether recording a new space in the system database should commit with the op that created it, so a refusal rolls back the record wherever the op ran.
- How this fits with [resuming an interrupted call after a crash](../../backlog/resume-after-crash.md), which covers the crash before an op's result commits.

## Testing

Data-driven crash tests, without mocks: for each place a space can be created, leave the files and records as they would be after the refused result committed and before cleanup, reopen CarbonDB, and check that only the system database and the spaces that settled ops account for remain.
