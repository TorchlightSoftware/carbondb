// [counter]: a table whose keys SQLite would make, which CarbonDB refuses.
export const setup = (db) => db.run('CREATE TABLE IF NOT EXISTS hit (id INTEGER PRIMARY KEY AUTOINCREMENT)')

export const calls = {}
