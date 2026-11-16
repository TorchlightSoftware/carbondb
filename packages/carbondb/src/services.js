// The services an app registers in main(), on every start. A service is a
// package: a WebAssembly component carrying its service.toml, as
// `carbondb/build` builds one, given as a file URL, a path or its bytes. The
// core reads its manifest, versions it by its bytes, compiles it, keeps the
// registry and routes to each service by name.

import {readFileSync} from 'node:fs'
import {core} from './native.js'

export class Services {
  #native

  constructor(native) {
    this.#native = native
  }

  add(name, component) {
    const bytes = bytesOf(component)
    core(() => this.#native.register(name, bytes))
  }
}

const bytesOf = (component) => (component instanceof Uint8Array ? component : readFileSync(component))
