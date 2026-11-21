// [roles]: each person's role, keyed by the person, which [notes] reads
// through `call` and `callsync`.
export const setup = (db) => db.run('CREATE TABLE IF NOT EXISTS role (person TEXT PRIMARY KEY, role TEXT NOT NULL) WITHOUT ROWID')

export const calls = {
  // Gives a person a role, replacing the one they held; looked up by key, as
  // a uniqueness check must be, never by a constraint.
  'POST [roles]/grants': {
    handler: ({input: {person, role}, env: {db}}) => {
      const held = db.get('SELECT role FROM role WHERE person = ?', [person])
      db.run(held ? 'UPDATE role SET role = ? WHERE person = ?' : 'INSERT INTO role (role, person) VALUES (?, ?)', [role, person])
    }
  },
  // One person's role, looked up by key, or null.
  'GET [roles]/roles/:person': {handler: ({path, env: {db}}) => db.get('SELECT role FROM role WHERE person = ?', [path.person])?.role ?? null},
  // Everyone's roles: a search, which `callsync` refuses.
  'GET [roles]/roles': {handler: ({env: {db}}) => db.all('SELECT person, role FROM role ORDER BY person')}
}
