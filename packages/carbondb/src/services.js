// The services an app registers in main(), on every start. A service is its
// exported object: its migrations and its calls, keyed by service key.

import {badRequest, notFound} from '@tlsft/service-error'
import {compileRoutes} from './routes.js'

// CarbonDB's own services, which an app never registers.
export const BUILTIN_SERVICES = ['carbon', 'space', 'identity', 'device']

export class Services {
  #registered = new Map()

  add(name, impl) {
    if (BUILTIN_SERVICES.includes(name)) throw new TypeError(`'${name}' is a CarbonDB service and cannot be registered`)
    if (this.#registered.has(name)) throw new TypeError(`A service '${name}' is already registered`)
    const migrations = impl.migrations ?? []
    if (!Array.isArray(migrations) || !migrations.every((migration) => typeof migration === 'string')) {
      throw new TypeError(`Service '${name}' declares migrations that are not a list of SQL strings`)
    }
    this.#registered.set(name, {name, migrations, routes: compileRoutes(impl.calls ?? {})})
  }

  get(name) {
    const service = this.#registered.get(name)
    if (!service) throw notFound(`Service '${name}' is not registered on this device`, {service: name})
    return service
  }

  // Refuse a new space naming a service this device cannot run.
  requireRegistered(names = []) {
    const missing = names.filter((name) => !BUILTIN_SERVICES.includes(name) && !this.#registered.has(name))
    if (missing.length) throw badRequest(`Services not registered on this device: ${missing.join(', ')}`, {services: missing})
  }
}
