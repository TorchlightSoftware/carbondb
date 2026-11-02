# A manager can add a service to an existing space

Status: backlog

## Idea

A space's services are fixed by its origin op today. Adding one can be another op on `[space]`, which needs manage like every other `[space]` POST. Calls to the new service are valid from that op's position in the shared order onward.

## To decide

- What a call to the service ordered before the add op does. The shared order makes the answer the same on every replica, rejected, but a late add op re-runs and so accepts calls that were rejected before it arrived.
- What a member's device does when it does not have the service installed. It cannot apply the ops, so the space pauses on that device until the app registers the service.
- Whether removing a service is the same op in reverse, and what happens to its tables.

## Related

- Story 5, where the personal space's services are named when the identity is created.
- Story 6, which defers adding services until a concrete use case.
