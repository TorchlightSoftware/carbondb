// CarbonDB for Node. An app's main() loads it with its TOML config, registers
// its services, and reaches its spaces through handles:
//
//   const carbon = await configure('./carbondb.toml')
//   carbon.services.add('chat', chat)
//   const room = carbon.ref('identity:alice').ref('personal:alice').ref('space:lobby')
//   await room.call('POST [chat]/messages', {text: 'hello'})

import {resolve} from 'node:path'
import {NativeCarbon, core} from './native.js'
import {Services} from './services.js'
import {Space} from './space.js'
import {SystemHandle} from './handle.js'

// One CarbonDB on this device: the core, the registered services, and the spaces opened so far.
class Carbon {
  #spaces = new Map()

  constructor(native) {
    this.native = native
    this.services = new Services(native)
  }

  space(id) {
    if (!this.#spaces.has(id)) this.#spaces.set(id, new Space(this, id))
    return this.#spaces.get(id)
  }
}

// Load CarbonDB from its TOML file, load the refs of every space on this
// device, so a handle reaches any of them at once, and return the system handle.
export async function configure(path) {
  const native = core(() => NativeCarbon.open(resolve(path)))
  const carbon = new Carbon(native)
  await Promise.all(core(() => native.spaces()).map((id) => carbon.space(id).load()))
  return new SystemHandle(carbon)
}
