// The services an app registers in main(), on every start. A service is a
// package: a WebAssembly component carrying its service.toml, as
// `carbondb/build` builds one, given as a file URL, a path or its bytes. The
// core reads its manifest, versions it by its bytes, compiles it off its own
// thread, keeps the registry and routes to each service by name. `add`
// settles once the service has compiled; calls to it made before then wait.

import {readFileSync} from 'node:fs'
import {answered} from './native.js'

export class Services {
  #native

  constructor(native) {
    this.#native = native
  }

  async add(name, component) {
    const bytes = bytesOf(component)
    return answered((reply) => this.#native.register(name, bytes, reply))
  }
}

const bytesOf = (component) => (component instanceof Uint8Array ? component : readFileSync(component))
