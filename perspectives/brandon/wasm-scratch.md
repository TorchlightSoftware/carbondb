    Still open: Replicated handlers' side effects are mediated by the WASM boundary:
    - Which replay policies a side-effect binding can declare, with a table of consequences for each. Proposed so far:
      - record, the default: the side effect happens on the first run only, and replays get the recorded responses;

"once" would be a clearer name.  "record" doesn't tell me anything about what it does.

      - repeat: made again on every run on every replica, which is safe only when the response feeds neither the database nor the output.

"replicate" lets say.  But aren't once and replicate mutually exclusive?  So are these tags, or are they an enum for a setting?

And do we imagine there being other configurations?  Whether we use enums or a tagging system, I want to ensure that we leave room for additions down the road and we don't back ourselves into a corner.

    - Whether a failed side effect is recorded like a response, so every replay sees the same failure the first run saw.

The outputs of the side effects have to be memoized and transmitted with the Op: I think we've already established that in our other discussions, yes?  The service itself is responsible for interpreting this and deciding whether it's a failure condition for the business logic.  Core's job is just to record it and provide it to the replicated services, not to interpret it.

---

   - Two services calling each other can deadlock until timeout. If A's op calls B and B's handler calls back into A, the call waits behind A's open transaction and times out.

Anything we can do about that?

   - A refused founding leaves orphan files. If POST [@carbon]/identity is refused after creating a channel's space, the system-database record rolls back but the file stays.

Can you clean that up?

   - Any service can post to a space it knows the id of, since the call effect accepts a space id. The target's own checks, such as manage, are the only guard.

Mmm... backlog?

   - Status reports this device's access. When no identity acts, which is now always the case on carbon, status reports manage if any identity on this device manages the channel.

I don't think you properly implemented devices if that's the case... devices are to be nested under identity... Hmm... maybe the whole identity concept needs to be implemented differently, outside of p2panda, so that it can be bootstrapped before we have a "device"?  I would much rather p2panda's sense of device be nested into our various identity sandboxes.

   - Some p2panda details are still open.

That's fine.

---

    identities are founded before and outside p2panda, and each one gets its own device key.

Uh... it gets a key for the current device that it's founded on, right?  But then can enroll others?

    [@carbon]'s calls about a channel or service (status, sync, delete) are made from that channel's handle and act as the handle's identity.

Ok.  So you would have to accept a param for service name, and otherwise give the status for the channel?

    Proposal: the call effect carries the chain of services waiting on it. A write posted into a service already in that chain is refused immediately, with 508 LOOP_DETECTED

Make it so.

    3. Orphan files: yes, I can clean them up.
    4. Calls by space id: backlog it.

Yep.

---

Update docs for 7 and 8.  Put 5 on the backlog.
