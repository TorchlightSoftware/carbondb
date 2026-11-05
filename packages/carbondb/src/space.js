// One space on this device, as the runtime holds it: calls go to the core,
// which routes and applies them, and watches re-run after each commit the core
// reports.

import {answered, core} from './native.js'
import {parseKey} from './routes.js'

export class Space {
  #watches = new Set()
  #flushing = false
  #listening = false

  constructor(carbon, id) {
    this.carbon = carbon
    this.id = id
  }

  // A sync lookup of the space a ref here points to.
  findRef(kind, slug) {
    return core(() => this.carbon.native.findRef(this.id, kind, slug))
  }

  async call(actor, key, input) {
    return (await this.#request(actor, key, input)).value
  }

  // A call's outcome: its value, and the tables it read or wrote.
  #request(actor, key, input = null) {
    const parts = parseKey(key)
    return answered((reply) => this.carbon.native.call(this.id, actor, parts, input, reply))
  }

  // Run a GET now and again after every commit that writes a table it read.
  // `fn` receives each result, or undefined and the error when the GET fails.
  watch(actor, key, fn) {
    this.#listen()
    const watch = {actor, key, fn, tables: new Set(), dirty: true, running: false, stopped: false}
    this.#watches.add(watch)
    this.#rerun(watch)
    return () => {
      watch.stopped = true
      this.#watches.delete(watch)
    }
  }

  // Hear the core's commits to this space, once.
  #listen() {
    if (this.#listening) return
    core(() => this.carbon.native.watch(this.id, (tables) => this.#changed(tables)))
    this.#listening = true
  }

  // Mark the watches a commit affects, and re-run them once the current batch of calls has settled.
  #changed(tables) {
    const written = new Set(tables)
    for (const watch of this.#watches) {
      if ([...watch.tables].some((table) => written.has(table))) watch.dirty = true
    }
    if (this.#flushing) return
    this.#flushing = true
    setImmediate(() => {
      this.#flushing = false
      for (const watch of this.#watches) this.#rerun(watch)
    })
  }

  async #rerun(watch) {
    if (!watch.dirty || watch.running || watch.stopped) return
    watch.dirty = false
    watch.running = true
    const {value, tables, error} = await this.#request(watch.actor, watch.key).catch((thrown) => ({error: thrown}))
    watch.running = false
    if (watch.stopped) return
    if (tables) watch.tables = new Set(tables)
    watch.fn(value, error)
    this.#rerun(watch)
  }
}
