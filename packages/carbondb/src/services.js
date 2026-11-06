// The services an app registers in main(), on every start. A service is its
// exported object: its `calls`, keyed by service key, and an optional `init`,
// which sets up its schema through its `db` binding. The core keeps the
// registry and routes to each service by name; registering hands it the
// service's version, its `sql` binding, its `init` and its entry point.

import {serializeError} from '@tlsft/service-error'
import {entryOf} from './entry.js'
import {core, hash, toError} from './native.js'
import {compileRoutes} from './routes.js'

// The one `sql` binding an npm service receives.
const BINDING = 'db'

export class Services {
  #native

  constructor(native) {
    this.#native = native
  }

  add(name, impl) {
    if (impl.init !== undefined && typeof impl.init !== 'function') throw new TypeError(`Service '${name}' declares an init that is not a function`)
    const entry = entryOf(compileRoutes(impl.calls ?? {}))
    core(() => this.#native.register(name, versionOf(impl), BINDING, initOf(name, impl), entry))
  }
}

// A service's version: the hash of its code, its handlers and its init, and of
// the migrations it exports, so different code is a different version.
function versionOf(impl) {
  const calls = Object.entries(impl.calls ?? {}).map(([key, call]) => [key, String(call.handler), call.tags ?? []])
  return hash({calls, init: impl.init ? String(impl.init) : null, migrations: impl.migrations ?? null})
}

// The core calls `init` with the service's `db` binding, open while it runs,
// and takes back null or a service error's data. `init` runs inside a
// transaction the core holds, so it finishes before it returns.
function initOf(name, impl) {
  return (binding) => {
    try {
      const returned = impl.init?.({
        run: (sql, params) => core(() => binding.run(sql, params)),
        get: (sql, params) => core(() => binding.get(sql, params)),
        all: (sql, params) => core(() => binding.all(sql, params))
      })
      if (typeof returned?.then === 'function') throw new TypeError(`Service '${name}' has an init that returns a Promise; init finishes before it returns`)
      return null
    } catch (thrown) {
      return serializeError(toError(thrown))
    }
  }
}
