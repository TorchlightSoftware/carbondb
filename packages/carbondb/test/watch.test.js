// When a watch must run again, checked against every interleaving of one run
// with up to three commits: before the run starts, while it is in flight, and
// after it answers, each writing a table the run read, one it did not, or both.

import {describe, it} from 'node:test'
import assert from 'node:assert'
import {committed, finished, outdated, started, unseen} from '../src/watch.js'

const WRITES = [['note'], ['tag'], ['note', 'tag']]
const READ_SETS = [[], ['note'], ['tag'], ['note', 'tag']]
const SLOTS = ['before', 'during', 'after']

// Every list of up to `n` commits, each with the tables it wrote and when it landed.
function commitLists(n) {
  if (n === 0) return [[]]
  const shorter = commitLists(n - 1)
  const longer = shorter.filter((list) => list.length === n - 1).flatMap((list) => WRITES.flatMap((tables) => SLOTS.map((slot) => [...list, {tables, slot}])))
  return [...shorter, ...longer]
}

// The watch after its previous answer, one run, and the commits around it.
function replay(previous, fresh, commits) {
  const at = (slot) => commits.filter((commit) => commit.slot === slot)
  let state = finished(started(unseen()), previous)
  for (const {tables} of at('before')) state = committed(state, tables)
  state = started(state)
  for (const {tables} of at('during')) state = committed(state, tables)
  state = finished(state, fresh)
  for (const {tables} of at('after')) state = committed(state, tables)
  return state
}

// The rule itself: run again exactly when a commit after the run started wrote
// a table the run read.
const model = (fresh, commits) => commits.some(({tables, slot}) => slot !== 'before' && tables.some((table) => fresh.includes(table)))

describe('watch', () => {
  it('runs again exactly when a commit after its run started wrote a table that run read', () => {
    let cases = 0
    for (const previous of READ_SETS) {
      for (const fresh of READ_SETS) {
        for (const commits of commitLists(3)) {
          assert.strictEqual(replay(previous, fresh, commits).stale, model(fresh, commits), JSON.stringify({previous, fresh, commits}))
          cases += 1
        }
      }
    }
    assert.strictEqual(cases, 16 * (1 + 9 + 81 + 729))
  })

  it('runs first, and keeps its read set when a run fails', () => {
    assert.strictEqual(unseen().stale, true)
    const read = finished(started(unseen()), ['note'])
    const failed = finished(committed(started(read), ['note']), undefined)
    assert.deepStrictEqual([...failed.read], ['note'])
    assert.strictEqual(failed.stale, true)
  })

  it('runs again whatever it read once outdated', () => {
    assert.strictEqual(outdated(finished(started(unseen()), [])).stale, true)
  })
})
