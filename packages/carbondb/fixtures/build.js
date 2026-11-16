// Build the test services, the fixtures the tests register, into components
// beside their sources, before the tests run. They live outside test/, where
// `node --test` would take every file for a test of its own.

import {readdirSync} from 'node:fs'
import {join} from 'node:path'
import {fileURLToPath} from 'node:url'
import {buildService} from '../src/build.js'

const here = fileURLToPath(new URL('.', import.meta.url))
const names = readdirSync(here, {withFileTypes: true}).filter((entry) => entry.isDirectory()).map((entry) => entry.name)
await Promise.all(names.map((name) => buildService({source: join(here, name, 'index.js'), manifest: join(here, name, 'service.toml'), out: join(here, name, `${name}.wasm`)})))
