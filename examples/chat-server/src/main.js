// The app's main(): load CarbonDB, register [chat], find or create the app's
// identity and its lobby, and serve the lobby over HTTP.
//
//   node src/main.js carbondb.toml [port]

import {createServer} from 'node:http'
import {once} from 'node:events'
import {fileURLToPath} from 'node:url'
import {configure} from '@tlsft/carbondb'
import * as chat from '@tlsft/chat-service'
import {serveSpace} from './http.js'

// This device's identity for the app, created on first start.
async function appIdentity(carbon, slug) {
  const known = (await carbon.listIdentities()).some((identity) => identity.slug === slug)
  if (known) return carbon.resumeIdentity(slug).ref(`personal:${slug}`)
  return (await carbon.createIdentity({slug, device: 'server', services: []})).personal
}

// The lobby the server answers for, created on first start.
async function lobby(personal) {
  const exists = (await personal.listSpaces()).some((ref) => ref.kind === 'space' && ref.slug === 'lobby')
  return exists ? personal.ref('space:lobby') : personal.createSpace({name: 'Lobby', services: ['chat']})
}

// Start the server; port 0 picks a free one. Resolves with its URL and a way to close it.
export async function start({config, port = 0}) {
  const carbon = await configure(config)
  carbon.services.add('chat', chat)
  const room = await lobby(await appIdentity(carbon, 'app'))
  const server = createServer(serveSpace(room))
  server.listen(port)
  await once(server, 'listening')
  const close = () => {
    server.closeAllConnections()
    return new Promise((resolve) => server.close(resolve))
  }
  return {url: `http://localhost:${server.address().port}`, room, close}
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [config = 'carbondb.toml', port = '3000'] = process.argv.slice(2)
  const {url} = await start({config, port: Number(port)})
  console.log(`[chat] listening on ${url}`)
}
