# napi-apply spike

Can the Rust core own a SQLite transaction while an async JS handler runs inside it? Yes, with everything on the JS thread.

```
npm test    # builds with cargo, then runs node:test
```

## What it shows

- `apply(handler)` opens a transaction, calls the handler, and chains commit and rollback onto the Promise the handler returns. Rust never blocks waiting for JS, so the handler's synchronous `run` and `all` calls re-enter Rust across its awaits and use the same connection, inside the open transaction.
- A rejection reaches the caller as the very value the handler threw, so a service error keeps its status, code and details.
- A handler that throws before returning a Promise, or a failing SQL statement, rolls back too.
- `read` answers from a second, read-only connection in WAL mode, and sees only committed state while a transaction is open.
- SQLite's authorizer reports the tables a query reads, which is the read set `watch` needs.

## What it found

- **napi-rs promise callbacks replace a thrown value with a fresh Error.** Returning `Err` from a `PromiseRaw::catch` callback throws a new Error built from the message, dropping a service error's fields. The rollback step instead returns `PromiseRaw::reject` with the original value, which JS flattens into the chained promise.
- **rusqlite's `serde_json` binding stores a string as JSON text,** quotes included. Parameters are converted to SQLite values directly, and only objects and arrays become JSON text.
- **The authorizer hook must be `Send`,** so the read set it fills sits behind an `Arc<Mutex>`.
- **One apply at a time per space.** A second `apply` while one is open is refused here; the real core queues it.

## Versions

napi 3.16.1, napi-derive 3.7.0, rusqlite 0.39.0 (`bundled`, `hooks`), Node 22.13.
