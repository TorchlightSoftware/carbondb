// When a watch must run again, as data. A watch runs its GET, learns from the
// answer which tables the GET read (its read set), and must run again after a
// commit that writes one of them.
//
// A run's snapshot is taken when it starts, but its read set is known only when
// it answers, so a commit landing in between cannot be judged on arrival. While
// a run is in flight, the tables such commits wrote are kept, and judged
// against the fresh read set when the run answers. The rule, then: a watch
// runs again exactly when a commit after its last run started wrote a table
// that run read.
//
// A watch's state: `read`, the read set of its last answer; `stale`, whether it
// must run again; and `inFlight`, the tables written since the running run's
// snapshot, or null when no run is in flight.

export const unseen = () => ({read: new Set(), stale: true, inFlight: null})

// A run starts: it reads everything committed so far.
export function started(state) {
  return {...state, stale: false, inFlight: new Set()}
}

// A commit wrote these tables.
export function committed(state, tables) {
  if (state.inFlight) return {...state, inFlight: new Set([...state.inFlight, ...tables])}
  return {...state, stale: state.stale || meets(state.read, tables)}
}

// The run in flight finished, with the tables it read; a failed GET reports
// none, and the last read set stands.
export function finished(state, tables) {
  const read = tables ? new Set(tables) : state.read
  return {read, stale: state.stale || meets(read, state.inFlight), inFlight: null}
}

// Something changed what every GET answers, such as the space stopping.
export const outdated = (state) => ({...state, stale: true})

const meets = (read, tables) => [...tables].some((table) => read.has(table))
