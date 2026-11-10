# An app developer can register the services the app runs

Status: agreed

## Calls

```js
import * as chat from './services/chat.js'

carbon.services.add('chat', chat)   // in main(), on every start
```

A service is its package: a `service.toml` and an exported module. The module exports `manifest`, where its `service.toml` is; its calls keyed by service key, each with a handler and optional tags (such as `append`); and an optional `init`, which sets up its schema through its `sql` binding on every open. A handler receives that binding in `env`, under the name its `service.toml` gives it. How it shapes its tables is its own concern: `@tlsft/carbondb/migrations` applies a directory of D1-style migrations from `init`. The name it runs under is the one passed to `services.add`, which overrides the name in its `service.toml`.

## Rules

- The app registers its services in `main()` on every start. Nothing is stored in the database, and the handler code is never copied into it.
- Nothing is loaded from hard-coded lists or the TOML file.
- A service's version is the hash of its package, carried in every op.
- A service without a `service.toml` is refused.
- A space naming a service this device has not registered is refused.
- Installing, updating and removing services at runtime is out of scope for the demo.

## Acceptance

- After a restart that registers [chat] again, a space using [chat] applies [chat] calls and still holds its earlier messages.
- Joining a space whose services include one not registered fails with an error naming it.
