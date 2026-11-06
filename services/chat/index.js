// [chat]: messages in a space. A service is its schema, set up by its `init`,
// and the handlers that read and write it; the app names it when it registers it:
//
//   carbon.services.add('chat', chat)
//
// Handlers take time and ids from the op, never from the clock or a random
// source, so every replica computes the same rows.

import {z} from 'zod'
import {forbidden, notFound, validationError} from '@tlsft/service-error'
import {applyMigrations, readMigrations} from '@tlsft/carbondb/migrations'

// Its migrations, D1-style files in migrations/, applied by `init` on every open.
export const migrations = readMigrations(new URL('./migrations/', import.meta.url))

export function init(db) {
  applyMigrations(db, migrations)
}

// What a message call takes. Validating input is the service's own business.
const MessageInput = z.object({text: z.string().trim().min(1).max(4000)}).strict()

function parse(schema, input) {
  const result = schema.safeParse(input)
  if (!result.success) throw validationError('Invalid message', result.error.issues.map(({path, message}) => ({path, message})))
  return result.data
}

// The live message an op may change: only its author may.
function ownMessage(db, id, author) {
  const message = db.get('SELECT author FROM message WHERE id = ? AND deleted_at IS NULL', [id])
  if (!message) throw notFound('No such message', {id})
  if (message.author !== author) throw forbidden('Only the author may change a message', {id})
  return message
}

export const calls = {
  // Posting only adds a message, so it never forces a rewind.
  'POST [chat]/messages': {
    tags: ['append'],
    handler: ({input, op, db}) => {
      const {text} = parse(MessageInput, input)
      const id = op.id()
      db.run('INSERT INTO message (id, author, text, sent_at) VALUES (?, ?, ?, ?)', [id, op.author, text, op.time])
      return {id}
    }
  },

  'PUT [chat]/messages/:id': {
    handler: ({path, input, op, db}) => {
      const {text} = parse(MessageInput, input)
      ownMessage(db, path.id, op.author)
      db.run('UPDATE message SET text = ?, edited_at = ? WHERE id = ?', [text, op.time, path.id])
      return {id: path.id}
    }
  },

  'DELETE [chat]/messages/:id': {
    handler: ({path, op, db}) => {
      ownMessage(db, path.id, op.author)
      db.run('UPDATE message SET deleted_at = ? WHERE id = ?', [op.time, path.id])
      return {id: path.id, deleted: true}
    }
  },

  // A read, answered from this device's copy; never an op.
  'GET [chat]/messages': {
    handler: ({db}) => db.all('SELECT id, author, text, sent_at, edited_at FROM message WHERE deleted_at IS NULL ORDER BY sent_at, id')
  }
}
