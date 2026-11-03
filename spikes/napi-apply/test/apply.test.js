import {describe, it, beforeEach} from 'node:test'
import assert from 'node:assert'
import {createRequire} from 'node:module'
import {mkdtempSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'

const {Space} = createRequire(import.meta.url)('../napi-apply.node')

// A ServiceError-shaped rejection, as a handler throws one.
const notFound = (message) => Object.assign(new Error(message), {status: 404, code: 'NOT_FOUND', details: {id: 'm1'}})

const tick = () => new Promise(resolve => setTimeout(resolve, 5))

let space
beforeEach(() => {
  space = new Space(join(mkdtempSync(join(tmpdir(), 'napi-apply-')), 'space.db'))
  space.run('CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT NOT NULL)')
  space.run('CREATE TABLE member (id TEXT PRIMARY KEY)')
})

const messages = () => space.read('SELECT id, text FROM message ORDER BY id').rows

describe('apply', () => {
  it('commits when an async handler resolves, after sync db calls across awaits', async () => {
    const result = await space.apply(async () => {
      space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m1', 'hello'])
      await tick()
      const [row] = space.all('SELECT text FROM message WHERE id = ?', ['m1'])
      await tick()
      space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m2', `${row.text} again`])
      return {id: 'm2'}
    })

    assert.deepStrictEqual(result, {id: 'm2'})
    assert.deepStrictEqual(messages(), [{id: 'm1', text: 'hello'}, {id: 'm2', text: 'hello again'}])
  })

  it('rolls back when the handler rejects, and the caller gets the same error', async () => {
    const thrown = notFound('No such message')
    await assert.rejects(
      space.apply(async () => {
        space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m1', 'hello'])
        await tick()
        throw thrown
      }),
      (error) => {
        assert.strictEqual(error, thrown)
        assert.deepStrictEqual({status: error.status, code: error.code, details: error.details}, {status: 404, code: 'NOT_FOUND', details: {id: 'm1'}})
        return true
      }
    )
    assert.deepStrictEqual(messages(), [])
  })

  it('rolls back when the handler throws before returning a Promise', async () => {
    assert.throws(() => space.apply(() => {
      space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m1', 'hello'])
      throw notFound('No such message')
    }), {code: 'NOT_FOUND'})
    assert.deepStrictEqual(messages(), [])
  })

  it('rolls back when a db call fails inside the handler', async () => {
    await assert.rejects(space.apply(async () => {
      space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m1', 'hello'])
      space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m1', 'duplicate'])
    }), /UNIQUE constraint failed/)
    assert.deepStrictEqual(messages(), [])
  })

  it('frees the space for the next apply after a commit and after a rollback', async () => {
    await space.apply(async () => space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m1', 'one']))
    await assert.rejects(space.apply(async () => { throw notFound('x') }))
    await space.apply(async () => space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m2', 'two']))
    assert.deepStrictEqual(messages().map(row => row.id), ['m1', 'm2'])
  })

  it('refuses a second apply while one is open; ordering them is the queue\'s job', async () => {
    const first = space.apply(async () => { await tick() })
    assert.throws(() => space.apply(async () => {}), /already open/)
    await first
  })
})

describe('read', () => {
  it('sees only committed state while a transaction is open', async () => {
    let seenDuring
    await space.apply(async () => {
      space.run('INSERT INTO message (id, text) VALUES (?, ?)', ['m1', 'hello'])
      await tick()
      seenDuring = messages()
    })
    assert.deepStrictEqual(seenDuring, [])
    assert.deepStrictEqual(messages(), [{id: 'm1', text: 'hello'}])
  })

  const readSets = [
    {sql: 'SELECT id, text FROM message', tables: ['message']},
    {sql: 'SELECT m.text FROM message m JOIN member p ON p.id = m.id', tables: ['member', 'message']},
    {sql: 'SELECT count(*) AS n FROM member', tables: ['member']}
  ]

  for (const {sql, tables} of readSets) {
    it(`reports the tables ${tables.join(', ')} read by ${sql}`, () => {
      assert.deepStrictEqual(space.read(sql).tables, tables)
    })
  }
})
