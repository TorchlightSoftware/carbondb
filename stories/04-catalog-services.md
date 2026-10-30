# An app developer can catalog the services the app runs

Status: agreed

## Calls

```js
await carbon.catalog.add('./services/chat.js')   // once
```

A service module exports its name, its migrations and its calls keyed by service key, each with a handler and optional tags (such as `append`).

## Rules

- Cataloging happens once and is stored in SQLite: name, version and module path. Later starts load the service from that path; the app repeats nothing.
- The path is relative to the TOML file. The handler code itself is never copied into the database.
- Nothing is loaded from hard-coded lists or the TOML file.
- A service's schema version is the number of its migrations, carried in every op.
- A space naming a service that is not in the catalog is refused.
- Installing, updating and removing services is out of scope for the demo.

## Acceptance

- After `catalog.add` and a restart, a space using [chat] applies [chat] calls without the app cataloging it again.
- Joining a space whose services include one not in the catalog fails with an error naming it.
