// One space on this device, as the runtime holds it: calls go to the core,
// which routes and applies them, and watches re-run after each commit the core
// reports. A call resolves only once the watches its commit touched have run
// again, so after `await call(...)` every watched view already shows the write.
//
// Each space keeps its refs from a watch on `GET [space]/refs`, so a handle
// finds a ref without waiting. `load()` starts it and resolves with its first
// answer; the runtime loads every space before handing out its handle.
//
// A space has a RunState: initializing until every service it names is set up
// here, idle, applying an op, or stopped by a system failure
// (its file or disk failed) until it is fixed by hand and resumed. While it is
// stopped every call rejects with a 503, and so every watch gets that error;
// once it resumes, every watch runs again.

import {internalError} from '@tlsft/service-error'
import {answered, core} from './native.js'
import {parseKey} from './routes.js'

// The actor the runtime reads refs as: none, since refs are open to anyone here.
const RUNTIME = {identity: null, device: null}

export class Space {
  #watches = new Set()
  #listening = false
  #runStateWatches = new Set()
  #hearingRunState = false
  #stopped = false
  #refreshing = Promise.resolve()
  #refs = null
  #loading = null

  constructor(carbon, id) {
    this.carbon = carbon
    this.id = id
  }

  // Load this space's refs and keep them current.
  load() {
    this.#loading ??= new Promise((resolve, reject) => {
      this.watch(RUNTIME, 'GET [space]/refs', (refs, error) => {
        if (error) return reject(error)
        this.#refs = refs
        resolve()
      })
    })
    return this.#loading
  }

  // The space a ref here points to, or null: read from this space's refs, which `load()` keeps.
  findRef(kind, slug) {
    if (!this.#refs) throw internalError(`The refs of space ${this.id} are not loaded`)
    return this.#refs.find((ref) => ref.kind === kind && ref.slug === slug) ?? null
  }

  // A call's value, once the watches its commit touched have run again.
  async call(actor, key, input) {
    const {value} = await this.#request(actor, key, input)
    await this.#refreshing
    return value
  }

  // A call's outcome: its value, and the tables it read or wrote.
  #request(actor, key, input = null) {
    const parts = parseKey(key)
    return answered((reply) => this.carbon.native.call(this.id, actor, parts, input, reply))
  }

  // Where this space is in applying its ops: `{state: 'initializing', services}`,
  // `{state: 'idle'}`, `{state: 'applying', op}`, or `{state: 'stopped', error}`.
  runState() {
    return core(() => this.carbon.native.runState(this.id))
  }

  // Call `fn` with each change of this space's RunState. Returns a function that stops it.
  watchRunState(fn) {
    this.#hearRunState()
    this.#runStateWatches.add(fn)
    return () => this.#runStateWatches.delete(fn)
  }

  // Resume this space once the system failure that stopped it is fixed.
  resume() {
    core(() => this.carbon.native.resume(this.id))
  }

  // Hear the core's RunState changes for this space, once.
  #hearRunState() {
    if (this.#hearingRunState) return
    core(() => this.carbon.native.watchRunState(this.id, (state) => this.#runStateChanged(state)))
    this.#hearingRunState = true
  }

  // Stopping and resuming change what every watch's GET answers, so each runs again.
  #runStateChanged(state) {
    for (const fn of this.#runStateWatches) deliver(fn, state)
    const stopped = state.state === 'stopped'
    if (stopped === this.#stopped) return
    this.#stopped = stopped
    for (const watch of this.#watches) watch.dirty = true
    this.#refreshing = Promise.all([...this.#watches].map((watch) => this.#rerun(watch)))
  }

  // Run a GET now and again after every commit that writes a table it read.
  // `fn` receives each result, or undefined and the error when the GET fails.
  watch(actor, key, fn) {
    this.#listen()
    const watch = {actor, key, fn, tables: new Set(), dirty: true, settling: null, stopped: false}
    this.#watches.add(watch)
    this.#rerun(watch)
    return () => {
      watch.stopped = true
      this.#watches.delete(watch)
    }
  }

  // Hear the core's commits to this space, and its stops and resumes, once.
  #listen() {
    if (this.#listening) return
    core(() => this.carbon.native.watch(this.id, (tables) => this.#changed(tables)))
    this.#hearRunState()
    this.#listening = true
  }

  // A commit: re-run the watches it touched, and hold calls until they have.
  #changed(tables) {
    const written = new Set(tables)
    for (const watch of this.#watches) {
      if ([...watch.tables].some((table) => written.has(table))) watch.dirty = true
    }
    this.#refreshing = Promise.all([...this.#watches].map((watch) => this.#rerun(watch)))
  }

  // Resolves once the watch has run against every commit that touched it.
  #rerun(watch) {
    watch.settling ??= this.#settle(watch).then(() => {
      watch.settling = null
      return watch.dirty && !watch.stopped ? this.#rerun(watch) : undefined
    })
    return watch.settling
  }

  async #settle(watch) {
    while (watch.dirty && !watch.stopped) {
      watch.dirty = false
      const {value, tables, error} = await this.#request(watch.actor, watch.key).catch((thrown) => ({error: thrown}))
      if (watch.stopped) return
      if (tables) watch.tables = new Set(tables)
      deliver(watch.fn, value, error)
    }
  }
}

// Hand a watch its result. A watch function that throws is the app's own
// failure: it surfaces as an uncaught error, as a throwing event handler's
// would, and never stalls the calls waiting on the watch.
function deliver(fn, ...results) {
  try {
    fn(...results)
  } catch (thrown) {
    queueMicrotask(() => {
      throw thrown
    })
  }
}
