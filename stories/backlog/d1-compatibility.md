# An app developer can run a Worker's D1 queries on CarbonDB unchanged

Status: backlog

## Why

Services ported from Cloudflare Workers are written against D1: `env.DB.prepare(sql).bind(...).run()`, `first()`, `all()`, `raw()`, `batch()`, `exec()`, and a result carrying `meta.last_row_id`. CarbonDB's `db` binding offers `run`, `get` and `all` instead, so a port has to rewrite every query. The [packages](../packages.md) plan already ports a Worker's `wrangler.toml` and its D1 migrations; its queries are the missing piece.

## Already agreed

- A row's key comes from the op that creates it, never from SQLite. An integer key is `DEFAULT (op_id())` in a `WITHOUT ROWID` table.
- The D1 layer reports the key `op_id()` drew as `meta.last_row_id`. SQLite's own last rowid is meaningless here: a `WITHOUT ROWID` table never sets it.

## Idea

A layer over a service's `db` binding offering D1's statement API and result shape. It fills the `meta` fields that mean something here, notably `last_row_id` and `changes`, and leaves the rest as D1 does when it has nothing to say.

## To decide

- Which runtimes offer it: the JS runtime first, and whether Rust services get one.
- What `last_row_id` is when a statement draws several keys from `op_id()`, or none.
- How `batch()` maps onto an op, which is already one transaction: likely a savepoint, so the batch stays atomic within it.
- How `init` uses it: D1's API returns promises, while `init` must finish before it returns.

## Testing

- A handler written against D1's API inserts a row keyed by `DEFAULT (op_id())`, and `meta.last_row_id` equals that row's key.
- `first`, `all`, `raw` and `batch` return D1's shapes, checked against a table of cases.
