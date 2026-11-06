# A person learns a call was rejected after merge

Status: backlog

## Idea

A call that applied on the author's device can be refused once ops from other devices land ahead of it in the shared order, or once a newer version of its service no longer supports it (see [old ops](old-ops.md)). Its writes are dropped, but the author is not told.

- The op stays in the space's op log, which records what was received. What happened when this device ran it is its result: a refused result carries the service error.
- Refusals are this device's own record, kept per space and never replicated. Every replica running the same version refuses the same op, including the author's device once its rewind re-runs it, so the author finds the refusal locally.
- The runtime records refusals. Services never write them: a replicated service cannot call a local one, so it simply throws.
- `[@error]`, a local service in every space, reads the refused results: `GET [@error]/all`, `GET [@error]/service/:name` and `GET [@error]/search`. An app watches them like any GET, so there is no separate notification API.
- A rewind re-runs ops and replaces their results, so an op refused once may apply on a re-run, and the reverse.

The person works through the refusals with the op history interface, where a revert is a compensating op.

## To decide

- Whether a refusal names the op that caused it.
- How a refusal is cleared once the person has dealt with it.
- What `GET [@error]/search` searches.
