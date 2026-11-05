// The services an app registers in main(), on every start. A service is its
// exported object: its migrations and its calls, keyed by service key. The core
// keeps the registry and routes to each service by name; registering hands it
// the service's migrations and its entry point.

import {entryOf} from './entry.js'
import {core} from './native.js'
import {compileRoutes} from './routes.js'

export class Services {
  #native

  constructor(native) {
    this.#native = native
  }

  add(name, impl) {
    const migrations = impl.migrations ?? []
    if (!Array.isArray(migrations) || !migrations.every((migration) => typeof migration === 'string')) {
      throw new TypeError(`Service '${name}' declares migrations that are not a list of SQL strings`)
    }
    const entry = entryOf(compileRoutes(impl.calls ?? {}))
    core(() => this.#native.register(name, migrations, entry))
  }
}
