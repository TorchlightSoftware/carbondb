import {describe, it} from 'node:test'
import assert from 'node:assert'
import {Queue} from '../src/queue.js'

const tick = () => new Promise((resolve) => setTimeout(resolve, 2))

describe('Queue', () => {
  it('runs tasks one at a time, in the order pushed', async () => {
    const queue = new Queue()
    const log = []
    const task = (name) => async () => {
      log.push(`${name} start`)
      await tick()
      log.push(`${name} end`)
      return name
    }
    const results = await Promise.all([queue.push(task('a')), queue.push(task('b'))])
    assert.deepStrictEqual(results, ['a', 'b'])
    assert.deepStrictEqual(log, ['a start', 'a end', 'b start', 'b end'])
  })

  it('keeps going after a task fails, and rejects only that task', async () => {
    const queue = new Queue()
    const failed = queue.push(async () => { throw new Error('first') })
    const next = queue.push(async () => 'second')
    await assert.rejects(failed, /first/)
    assert.strictEqual(await next, 'second')
  })
})
