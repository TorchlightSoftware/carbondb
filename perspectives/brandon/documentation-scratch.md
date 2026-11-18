
Can you restate what the final system landed in terms of ops and database locks?  In terms of:

- local services
- replicated services
- side effects
- errors and results

Is this already documented somewhere?  I propose to document it in docs/runtime.md.

---

Can you restate where we landed in terms of cryptographic primitives, and how we mediate access to them?  Walk me through what happens in our identity creation, device enrollment, chat scenario in terms of cryptographic key creation and signing/verification.

Is this already documented somewhere?  I propose to document it in docs/crypto.md.

---

Can you discuss gaps between our notion of identity, devices, channels and membership, and p2panda's notion of devices, spaces, groups, and membership?  What does this reveal?  Are there opportunities to more closely embrace p2panda and eliminate complexity on our end?  Is there justification for us requesting changes upstream to p2panda that would make our implementation more seamless without digressing too far from their stated architectural goals?

Is this already documented somewhere?  I propose to document it in docs/p2panda-gaps.md.
