// The services an app registers in main(), on every start. A service is its
// exported module: `manifest`, where its package's service.toml is; its
// `calls`, keyed by service key; and an optional `init`, which sets up its
// schema through its `db` binding. The core reads the manifest, versions the
// package, keeps the registry and routes to each service by name; registering
// hands it the manifest's path, the service's `init` and its entry point.

import {fileURLToPath} from 'node:url'
import {serializeError} from '@tlsft/service-error'
import {entryOf} from './entry.js'
import {core, toError} from './native.js'
import {compileRoutes} from './routes.js'

export class Services {
  #native

  constructor(native) {
    this.#native = native
  }

  add(name, impl) {
    if (impl.init !== undefined && typeof impl.init !== 'function') throw new TypeError(`Service '${name}' declares an init that is not a function`)
    const entry = entryOf(compileRoutes(impl.calls ?? {}))
    core(() => this.#native.register(name, pathOf(impl.manifest), initOf(name, impl), entry))
  }
}

// Where a manifest is, as the core reads it: from a file URL, as
// `new URL('./service.toml', import.meta.url)` gives, or a path; null when
// the service exports none, which the core refuses.
const pathOf = (manifest) => (manifest instanceof URL ? fileURLToPath(manifest) : manifest ?? null)

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
