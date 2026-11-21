// [notes]: a small service exercising what a component offers a handler.

import {conflict} from '@tlsft/service-error'
import {applyMigrations} from 'carbondb/migrations'
import note from './migrations/0001_note.sql'
import tag from './migrations/0002_tag.sql'

export const migrations = [{name: '0001_note', sql: note}, {name: '0002_tag', sql: tag}]

export const setup = (db) => applyMigrations(db, migrations)

const insert = (db, op, text) => db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [op.id(), op.author, text, op.time])

export const calls = {
  // Writes a note twice, refusing in between when asked.
  'POST [notes]/notes': {
    handler: async ({input, op, env: {db}}) => {
      const id = op.id()
      db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [id, op.author, input.text, op.time])
      await Promise.resolve()
      if (input.refuse) throw conflict('Refused after writing', {id})
      insert(db, op, `${input.text} again`)
      return {id, author: op.author, device: op.device}
    }
  },
  // Writes, then computes forever, outliving its timeout.
  'POST [notes]/spin': {
    handler: ({input, op, env: {db}}) => {
      insert(db, op, input.text)
      for (;;) {}
    }
  },
  'POST [notes]/duplicates': {
    handler: ({env: {db}}) => {
      db.run("INSERT INTO note (id, author, text, at) VALUES ('same', 'a', 'x', 0)")
      db.run("INSERT INTO note (id, author, text, at) VALUES ('same', 'a', 'x', 0)")
    }
  },
  // Writes a note, reads through a call inside the op, then optionally makes
  // another call and refuses: the shapes a nested transaction must get right.
  'POST [notes]/nested': {
    handler: async ({input, op, env: {db}, call}) => {
      insert(db, op, input.text)
      const seen = (await call('GET [notes]/notes')).map((row) => row.text)
      if (input.inner) await call('POST [notes]/notes', input.inner).catch((error) => { if (!input.catch) throw error })
      if (input.refuse) throw conflict('Refused after its calls')
      return {seen}
    }
  },
  // Reach [roles] through `call` or `callsync`, as `input.through` says:
  // write a note only when a person reads as the bookkeeper there; answer a
  // search of everyone's roles; or grant a role there, then refuse.
  'POST [notes]/checked': {
    handler: async ({input, op, env: {db}, ...reach}) => {
      const role = await reach[input.through](`GET [roles]/roles/${input.person}`)
      if (role !== 'bookkeeper') throw conflict(`${input.person} is not the bookkeeper`, {role})
      insert(db, op, `checked ${input.person}`)
    }
  },
  'POST [notes]/searched': {handler: ({input, ...reach}) => reach[input.through]('GET [roles]/roles')},
  'POST [notes]/granted': {
    handler: async ({input, ...reach}) => {
      await reach[input.through]('POST [roles]/grants', {person: input.person, role: 'bookkeeper'})
      throw conflict('Refused after granting')
    }
  },
  // An effect this device cannot carry out alone.
  'POST [notes]/reach': {handler: ({effect}) => effect('invite', {contact: 'sally'})},
  'POST [notes]/unawaited': {
    handler: ({call}) => {
      call('POST [notes]/notes', {text: 'late'}).catch(() => {})
    }
  },
  // What the clock and randomness show a handler: for a write, the op's time
  // and randomness drawn from the op.
  'POST [notes]/clock': {handler: ({op}) => ({now: Date.now(), time: op.time, random: Math.random()})},
  'GET [notes]/clock': {handler: () => ({now: Date.now()})},
  'POST [notes]/logged': {
    handler: ({input}) => {
      console.log('logged', input)
      return {logged: true}
    }
  },
  'GET [notes]/notes': {handler: ({env: {db}}) => db.all('SELECT text FROM note ORDER BY at, text')},
  'GET [notes]/writes': {handler: ({call}) => call('POST [notes]/tags', {name: 'red'})},
  'POST [notes]/tags': {handler: ({input, env: {db}}) => db.run('INSERT INTO tag (name) VALUES (?)', [input.name])},
  'GET [notes]/broken': {handler: ({env: {db}}) => db.all('SELECT * FROM nowhere')}
}
