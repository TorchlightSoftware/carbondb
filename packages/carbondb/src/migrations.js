// An opt-in helper for a service's `setup`: apply D1-style migrations, each
// named, in order, tracking the ones applied in the service's own
// `d1_migrations` table, as Cloudflare D1 does. A service ported from a Worker
// keeps its migrations directory, imports each file as text, and applies
// them from `setup`:
//
//   import {applyMigrations} from 'carbondb/migrations'
//   import createMessages from './migrations/0001_create_messages.sql'
//   const migrations = [{name: '0001_create_messages', sql: createMessages}]
//   export const setup = (db) => applyMigrations(db, migrations)
//
// It uses nothing but the `db` binding, so it runs wherever the service does.

// The tracking table. Its key is the migration's name, so it never takes a key from SQLite.
const TRACKING = 'CREATE TABLE IF NOT EXISTS d1_migrations (name TEXT PRIMARY KEY) WITHOUT ROWID'

// Apply the migrations not applied yet. Schema only moves forward: a database
// that has applied a migration this code does not declare was set up by newer
// code, and is refused.
export function applyMigrations(db, migrations) {
  db.run(TRACKING)
  const applied = new Set(db.all('SELECT name FROM d1_migrations').map((row) => row.name))
  const declared = new Set(migrations.map((migration) => migration.name))
  const unknown = [...applied].find((name) => !declared.has(name))
  if (unknown) throw new Error(`This database has applied migration '${unknown}', which this version of the service does not declare`)
  for (const {name, sql} of migrations.filter((migration) => !applied.has(migration.name))) {
    db.run(sql)
    db.run('INSERT INTO d1_migrations (name) VALUES (?)', [name])
  }
}
