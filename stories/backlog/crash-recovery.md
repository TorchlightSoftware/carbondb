# A call a crash interrupts leaves a consistent result

Status: backlog

A crash can interrupt a call at two places: before its result commits, and after it commits but before the core finishes cleaning up what a refused run made. Both should leave the same state as a call that was never interrupted.

## Resuming an op without a result

The op log records each op as it arrives, and an op without a result runs again when its service is next ready. One test covers this, a `[notes]` op logged before a crash (`applies_an_op_left_without_a_result_once_its_service_is_registered_again` in `crates/core/src/tests/oplog_entries.rs`). Calls that build things in several steps aren't covered, and at least one fails: if the app crashes during `POST [@carbon]/identity` after the file of one of the identity's services is created but before the call's result commits, the call runs again on restart, creating that service refuses because its file exists (`ServiceSpace::create` in `crates/core/src/channels/service.rs`), and the identity is left half made with a refused result.

Each step of a multi-step call should find what an earlier run already built and carry on, rather than refusing, so the result is the same whether or not the call was interrupted. Whether a run that resumes can derive the ids an earlier run reached depends on whether p2panda assigns them, which is open ([p2panda-sync.md](../p2panda-sync.md)).

## Cleaning up after a refused op

When an op that created a channel or service is refused, the core deletes what its run created, and what their own ops created in turn (`Carbon::unmake` in `crates/core/src/channels/mod.rs`). That cleanup runs after the refused result commits, so a crash between the two leaves the files behind: the op is settled and never runs again, so nothing deletes them.

What is left depends on where the service was recorded. One created inside the system's own op, such as founding an identity, is recorded in the system database in that op's transaction, so a refusal rolls the record back and leaves only an unrecorded file. One created by a service's op, such as `POST [channel]/services`, is recorded in the system database at once, outside the caller's transaction, so it stays recorded as held by this device, though no channel names it. Retrying is not blocked: a new op derives new ids, so the leftovers are only wasted disk and, for a recorded one, a service opened on every start.

Cleanup could become something the device finishes on its next start rather than something it must finish before a crash: on open, the core finds what no settled op still accounts for and deletes it, the same way whichever op created it.

## To decide

- How a step tells a service it already built from one that belongs to someone else.
- Whether a resumed op's calls into other services, which may have committed before the crash, run twice.
- How the core tells a leftover from a service still in use: a file with no record in the system database, a record no channel names, or the `create` effects recorded on refused ops.
- Whether recording a new service in the system database should commit with the op that created it, so a refusal rolls back the record wherever the op ran.

## Testing

Data-driven crash tests, without mocks: for each step of a call that builds things (`POST [@carbon]/identity`, `POST [channel]/channel`, `POST [channel]/services`), leave the files and records as they would be after that step, reopen CarbonDB, and check that the call finishes, or is cleaned up, with the same result and state as an uninterrupted one.
