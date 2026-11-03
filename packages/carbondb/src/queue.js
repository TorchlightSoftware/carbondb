// Tasks run one at a time, in the order they were pushed. A space has one queue
// for writes and one for reads, so a transaction never overlaps another on the
// same connection. A failed task does not stop the ones after it.
export class Queue {
  #tail = Promise.resolve()

  push(task) {
    const run = this.#tail.then(task)
    this.#tail = run.catch(() => {})
    return run
  }
}
