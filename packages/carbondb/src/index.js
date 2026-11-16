// CarbonDB for Node. An app's main() loads it with its TOML config, registers
// its services, and reaches its channels through handles:
//
//   const carbon = await configure('./carbondb.toml')
//   carbon.services.add('chat', chat)
//   const lobby = carbon.ref('identity:alice').ref('personal:alice').ref('channel:lobby')
//   await lobby.call('POST [chat]/messages', {text: 'hello'})
//
// Everything CarbonDB keeps is in the core; this package hands it calls and
// hosts the app's npm services.

import {resolve} from 'node:path'
import {NativeCarbon, core} from './native.js'
import {Services} from './services.js'
import {SystemHandle} from './handle.js'

// Load CarbonDB from its TOML file and return the system handle.
export async function configure(path) {
  const native = core(() => NativeCarbon.open(resolve(path)))
  return new SystemHandle(native, new Services(native))
}
