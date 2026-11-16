#!/usr/bin/env node
// Build a JavaScript service into a package, a component carrying its service.toml:
//
//   carbondb-build <service.js> <service.toml> <out.wasm>

import {buildService} from '../src/build.js'

const [source, manifest, out] = process.argv.slice(2)
if (!source || !manifest || !out) {
  console.error('usage: carbondb-build <service.js> <service.toml> <out.wasm>')
  process.exit(2)
}
await buildService({source, manifest, out})
