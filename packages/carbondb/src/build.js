// Build a JavaScript service into a package: a WebAssembly component of the
// `carbondb:service` world, carrying its service.toml. The service is bundled
// with its dependencies into one module, by the Rolldown jco ships: a `.sql`
// file imports as its text, so a service keeps D1-style migration files, and
// the world's own interfaces stay imports of the component. jco then compiles
// the module with QuickJS (`jco componentize --backend quickjs`); the
// component runs the service through `carbondb/service`, and reaches nothing
// but the world and the WASI its host links. jco is the service's own build
// dependency.
//
//   import {buildService} from 'carbondb/build'
//   await buildService({source: './index.js', manifest: './service.toml', out: './chat.wasm'})

import {execFile} from 'node:child_process'
import {mkdtemp, readFile, rm, writeFile} from 'node:fs/promises'
import {createRequire} from 'node:module'
import {tmpdir} from 'node:os'
import {basename, dirname, join, resolve} from 'node:path'
import {fileURLToPath, pathToFileURL} from 'node:url'
import {promisify} from 'node:util'
import {packageComponent} from './native.js'

const here = dirname(fileURLToPath(import.meta.url))
const WIT = join(here, '..', 'wit')

// jco's command line, and the Rolldown it ships.
const jco = () => fileURLToPath(new URL('jco.js', import.meta.resolve('@bytecodealliance/jco')))
const rolldown = async () => import(pathToFileURL(createRequire(jco()).resolve('rolldown')).href)

// The entry jco builds: the service's module, served as the world's exports.
const entryOf = (source) => `import * as service from './${basename(source)}'
import {serve} from 'carbondb/service'
export const {setup, handle} = serve(service)
`

// Build the service whose module is `source`, with its `manifest`, into `out`.
export async function buildService({source, manifest, out}) {
  const entry = join(dirname(resolve(source)), '.carbondb-entry.js')
  const working = await mkdtemp(join(tmpdir(), 'carbondb-build-'))
  const [bundled, built] = [join(working, 'service.js'), join(working, 'component.wasm')]
  await writeFile(entry, entryOf(source))
  try {
    await bundle(entry, bundled)
    await componentize(bundled, built)
    const packaged = packageComponent(await readFile(built), await readFile(manifest, 'utf8'))
    await writeFile(out, packaged)
  } finally {
    await rm(entry, {force: true})
    await rm(working, {recursive: true, force: true})
  }
}

// The service and its dependencies as one ES module.
async function bundle(entry, out) {
  const {rolldown: build} = await rolldown()
  const bundled = await build({input: entry, platform: 'neutral', moduleTypes: {'.sql': 'text'}, external: [/^carbondb:/, /^wasi:/]})
  await bundled.write({file: out, format: 'esm'})
  await bundled.close()
}

async function componentize(source, out) {
  const args = [jco(), 'componentize', source, '--wit', WIT, '--world-name', 'service', '--backend', 'quickjs', '-o', out]
  await promisify(execFile)(process.execPath, args, {cwd: dirname(source)})
}
