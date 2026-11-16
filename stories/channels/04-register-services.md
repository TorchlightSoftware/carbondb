# An app developer can register the services the app runs

Status: implemented — single-node

## Calls

```js
import * as chat from './services/chat.js'

carbon.services.add('chat', chat)   // in main(), on every start
```

A service is its package: a `service.toml` and an exported module. The module exports `manifest`, where its `service.toml` is; its calls keyed by service key, each with a handler and optional tags (such as `append`); and an optional `init`, which sets up its schema through its `sql` binding on every open. A handler receives that binding in `env`, under the name its `service.toml` gives it, and `call` and `effect` for what it does outside its own tables. How it shapes its tables is its own concern: `carbondb/migrations` applies a directory of D1-style migrations from `init`. The name it is registered under is the one passed to `services.add`, which overrides the name in its `service.toml`; a channel names each service it runs by that registered package (see [story 6](06-create-shared-channel.md)).

## Rules

- The app registers its services in `main()` on every start. Nothing is stored in the database, and the handler code is never copied into it.
- Nothing is loaded from hard-coded lists or the TOML file.
- A service's version is the hash of its package, carried in every op.
- A service without a `service.toml` is refused with a 400, as is one declaring a field CarbonDB does not know or more than one `sql` binding.
- A name already registered, CarbonDB's own (`channel`, `identity`) included, is refused with a 409.
- Each service runs in a space of its own: one SQLite file beside the system database, holding its tables, its op log and its results. Its statements run as written, and it may not touch CarbonDB's `carbon_` tables or attach another file.
- Creating a service, with `POST [channel]/services`, for a package this device has not registered is refused with a 404 naming it; so is founding an identity whose `services` name one, with a 400. A service that already exists and runs one opens initializing: its GETs get a 404, and its ops wait until the package is registered.
- Installing, updating and removing services at runtime is out of scope for the demo.

## Acceptance

- After a restart that registers [chat] again, a channel naming [chat] applies [chat] calls and still holds its earlier messages.
- After a restart that does not register it, `GET [chat]/messages` gets a 404 naming it, and `runState('chat')` on the channel's handle is `{state: 'initializing', services: ['chat']}`.
- Joining a channel that names a service not registered here: needs peers until p2panda. Whether the join fails naming it, or the service opens initializing as after a restart, is not yet defined.
