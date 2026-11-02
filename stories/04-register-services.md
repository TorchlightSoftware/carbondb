# An app developer can register the services the app runs

Status: agreed

## Calls

```js
import * as chat from './services/chat.js'

carbon.services.add('chat', chat)   // in main(), on every start
```

A service is its exported object: its migrations and its calls keyed by service key, each with a handler and optional tags (such as `append`). The name it runs under is the one passed to `services.add`.

## Rules

- The app registers its services in `main()` on every start. Nothing is stored in the database, and the handler code is never copied into it.
- Nothing is loaded from hard-coded lists or the TOML file.
- A service's schema version is the number of its migrations, carried in every op.
- A space naming a service this device has not registered is refused.
- Installing, updating and removing services at runtime is out of scope for the demo.

## Acceptance

- After a restart that registers [chat] again, a space using [chat] applies [chat] calls and still holds its earlier messages.
- Joining a space whose services include one not registered fails with an error naming it.
