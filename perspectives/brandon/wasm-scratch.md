    Still open: Replicated handlers' side effects are mediated by the WASM boundary:
    - Which replay policies a side-effect binding can declare, with a table of consequences for each. Proposed so far:
      - record, the default: the side effect happens on the first run only, and replays get the recorded responses;

"once" would be a clearer name.  "record" doesn't tell me anything about what it does.

      - repeat: made again on every run on every replica, which is safe only when the response feeds neither the database nor the output.

"replicate" lets say.  But aren't once and replicate mutually exclusive?  So are these tags, or are they an enum for a setting?

And do we imagine there being other configurations?  Whether we use enums or a tagging system, I want to ensure that we leave room for additions down the road and we don't back ourselves into a corner.

    - Whether a failed side effect is recorded like a response, so every replay sees the same failure the first run saw.

The outputs of the side effects have to be memoized and transmitted with the Op: I think we've already established that in our other discussions, yes?  The service itself is responsible for interpreting this and deciding whether it's a failure condition for the business logic.  Core's job is just to record it and provide it to the replicated services, not to interpret it.
