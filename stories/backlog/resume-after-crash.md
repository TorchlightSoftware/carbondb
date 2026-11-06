# A person's interrupted call finishes when the app restarts after a crash

Status: backlog

## Why

The op log records each op as it arrives, and an op without a result runs again when its space is ready. One test covers this, a `[notes]` op logged before a crash (`applies_an_op_left_without_a_result_when_its_space_opens_again` in `crates/core/src/tests.rs`). Calls that build things in several steps aren't covered, and at least one fails: if the app crashes during `POST [@carbon]/identity` after the identity space's file is created but before the call's result commits, the call runs again on restart, creating that space refuses because its file exists (`crates/core/src/space.rs`, `Space::create`), and the identity is left half made with a refused result.

## Idea

A space's id is the hash of the op that creates it, so a call that runs again reaches the same space. Each step of a multi-step call finds what an earlier run already built and carries on, rather than refusing. The result is the same whether or not the call was interrupted.

## To decide

- How a step tells a space it already built from one that belongs to someone else.
- Whether a resumed op's calls into other spaces, which may have committed before the crash, run twice.
- How ops logged at the end of a turn while another op held the file (an open topic) fit in.

## Testing

Data-driven crash tests, without mocks: for each step of `[@carbon]/identity` and `[@carbon]/space`, leave the files as they would be after that step, reopen CarbonDB, and check that the call finishes with the same result and state as an uninterrupted one.
