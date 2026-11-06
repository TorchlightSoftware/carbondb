// [chat] on single-node CarbonDB, called the way an app calls it: through a
// space handle, with no adapter in between.

import {describe, it, beforeEach} from 'node:test'
import assert from 'node:assert'
import {mkdtempSync, writeFileSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {configure} from '@tlsft/carbondb'
import * as chat from '../index.js'

function configFile() {
  const path = join(mkdtempSync(join(tmpdir(), 'chat-')), 'carbondb.toml')
  writeFileSync(path, 'system = "data/system.db"\n[sync]\nmode = "none"\n')
  return path
}

// An app's start: load CarbonDB, register [chat], and reach the lobby, made on
// the first start and found again on later ones.
async function start(config) {
  const carbon = await configure(config)
  carbon.services.add('chat', chat)
  const known = (await carbon.listIdentities()).some((identity) => identity.slug === 'app')
  if (known) return carbon.resumeIdentity('app').ref('personal:app').ref('space:lobby')
  const {personal} = await carbon.createIdentity({slug: 'app', device: 'server', services: []})
  return personal.createSpace({name: 'Lobby', services: ['chat']})
}

let config, lobby
beforeEach(async () => {
  config = configFile()
  lobby = await start(config)
})

const messages = () => lobby.call('GET [chat]/messages')

describe('messages', () => {
  it('posts, lists, edits and deletes a message', async () => {
    const {id} = await lobby.call('POST [chat]/messages', {text: 'hello'})

    const [listed] = await messages()
    assert.deepStrictEqual([listed.id, listed.text, listed.edited_at], [id, 'hello', null])
    assert.match(listed.author, /^[0-9a-f]{64}$/)

    assert.deepStrictEqual(await lobby.call(`PUT [chat]/messages/${id}`, {text: 'hello, edited'}), {id})
    const [edited] = await messages()
    assert.strictEqual(edited.text, 'hello, edited')
    assert.ok(edited.edited_at >= listed.sent_at)

    assert.deepStrictEqual(await lobby.call(`DELETE [chat]/messages/${id}`), {id, deleted: true})
    assert.deepStrictEqual(await messages(), [])
  })

  it('lists messages in the order they were sent', async () => {
    for (const text of ['one', 'two', 'three']) await lobby.call('POST [chat]/messages', {text})
    assert.deepStrictEqual((await messages()).map((message) => message.text), ['one', 'two', 'three'])
  })

  it('keeps messages and the identity across a restart', async () => {
    await lobby.call('POST [chat]/messages', {text: 'still here'})
    const [before] = await messages()
    lobby = await start(config)
    const [after] = await messages()
    assert.deepStrictEqual(after, before)
  })
})

describe('errors', () => {
  const cases = [
    {why: 'an empty message', key: 'POST [chat]/messages', input: {text: '  '}, status: 422, code: 'VALIDATION_ERROR'},
    {why: 'a message with no text', key: 'POST [chat]/messages', input: {}, status: 422, code: 'VALIDATION_ERROR'},
    {why: 'a message with unknown fields', key: 'POST [chat]/messages', input: {text: 'x', pinned: true}, status: 422, code: 'VALIDATION_ERROR'},
    {why: 'editing a message that does not exist', key: 'PUT [chat]/messages/nope', input: {text: 'x'}, status: 404, code: 'NOT_FOUND'},
    {why: 'deleting a message that does not exist', key: 'DELETE [chat]/messages/nope', status: 404, code: 'NOT_FOUND'}
  ]

  for (const {why, key, input, status, code} of cases) {
    it(`refuses ${why} with ${status} ${code}`, async () => {
      await assert.rejects(lobby.call(key, input), {status, code})
    })
  }

  it('reports what failed validation in the details', async () => {
    const error = await lobby.call('POST [chat]/messages', {}).catch((thrown) => thrown)
    assert.deepStrictEqual(error.details.map((issue) => issue.path), [['text']])
  })

  it('leaves no message behind after a refused call', async () => {
    await assert.rejects(lobby.call('POST [chat]/messages', {text: ''}))
    assert.deepStrictEqual(await messages(), [])
  })
})

describe('status', () => {
  it('describes the lobby: its name, services and single-node state', async () => {
    const {kind, name, services, access, sync, peers} = await lobby.status()
    assert.deepStrictEqual({kind, name, services, access, sync, peers}, {kind: 'space', name: 'Lobby', services: ['chat'], access: ['manage'], sync: 'off', peers: []})
  })
})

describe('watch', () => {
  it('lists the messages again after each message is posted', async () => {
    const seen = []
    let first
    const listed = new Promise((resolve) => { first = resolve })
    const stop = lobby.watch('GET [chat]/messages', (value) => {
      seen.push(value.map((message) => message.text))
      first()
    })
    await listed
    await lobby.call('POST [chat]/messages', {text: 'live'})
    stop()
    assert.deepStrictEqual(seen, [[], ['live']])
  })
})
