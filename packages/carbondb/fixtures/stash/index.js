// [stash]: its binding is named `store`.
export const setup = (db) => db.run('CREATE TABLE IF NOT EXISTS item (id TEXT PRIMARY KEY, name TEXT NOT NULL) WITHOUT ROWID')

export const calls = {
  'POST [stash]/items': {handler: ({input, op, env}) => env.store.run('INSERT INTO item (id, name) VALUES (?, ?)', [op.id(), input.name])},
  'GET [stash]/items': {handler: ({env}) => ({bindings: Object.keys(env), items: env.store.all('SELECT name FROM item')})}
}
