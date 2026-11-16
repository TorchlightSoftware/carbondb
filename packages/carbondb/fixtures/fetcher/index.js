// [fetcher]: fetches from the world outside with the global `fetch`, all its
// requests at once; a write notes what came back. `race` shows the handler
// running on while a request waits.
export const setup = (db) => db.run('CREATE TABLE IF NOT EXISTS fetched (id TEXT PRIMARY KEY, answers TEXT NOT NULL) WITHOUT ROWID')

const fetchAll = (requests) => Promise.all(requests.map((request) => fetchOne(request).catch((error) => ({error: `${error.name}: ${error.message}`}))))

async function fetchOne({url, ...init}) {
  const response = await fetch(url, init)
  return {status: response.status, ok: response.ok, type: response.headers.get('content-type'), text: await response.text()}
}

export const calls = {
  'POST [fetcher]/fetch': {
    handler: async ({input, op, env}) => {
      const answers = await fetchAll(input.requests)
      env.db.run('INSERT INTO fetched (id, answers) VALUES (?, ?)', [op.id(), JSON.stringify(answers)])
      return answers
    }
  },
  'GET [fetcher]/fetch': {handler: ({input}) => fetchAll(input.requests)},
  // Starts the slow request, then awaits the fast one: the order they finished in.
  'GET [fetcher]/race': {
    handler: async ({input}) => {
      const finished = []
      const slow = fetch(input.slow).then(() => finished.push('slow'))
      await fetch(input.fast)
      finished.push('fast')
      await slow
      return finished
    }
  }
}
