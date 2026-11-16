// Builds the editor into one script, its stylesheet and its page, in the
// resources of the app (`apps/NiceMailer/Sources/NiceMailer/Resources/Editor`,
// ignored by git: the bundle is built, never committed). `--out <dir>`
// writes elsewhere (a web client).
//
// The licences of the dependencies are written beside them
// (`THIRD_PARTY_LICENSES.txt`), as the app does for Dark Reader.

import { build } from 'esbuild'
import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const at = process.argv.indexOf('--out')
const out = at > 0 ? process.argv[at + 1] : join(here, '../NiceMailer/Sources/NiceMailer/Resources/Editor')
mkdirSync(out, { recursive: true })

await build({
  entryPoints: [join(here, 'src/index.ts')],
  bundle: true,
  minify: true,
  format: 'iife',
  target: ['safari17'],
  outfile: join(out, 'editor.js'),
  legalComments: 'none',
  logLevel: 'warning',
})
copyFileSync(join(here, 'src/editor.css'), join(out, 'editor.css'))
copyFileSync(join(here, 'src/index.html'), join(out, 'index.html'))

// The licence of each package bundled: every dependency, and theirs.
const lock = JSON.parse(readFileSync(join(here, 'package-lock.json'), 'utf8'))
const notices = []
for (const [path, info] of Object.entries(lock.packages ?? {})) {
  if (!path || info.dev) continue
  const dir = join(here, path)
  const file = existsSync(dir) ? readdirSync(dir).find(f => /^licen[cs]e/i.test(f)) : undefined
  const text = file ? readFileSync(join(dir, file), 'utf8').trim() : `License: ${info.license ?? 'unknown'}`
  notices.push(`${path.replace(/^node_modules\//, '')} ${info.version}\n\n${text}`)
}
writeFileSync(join(out, 'THIRD_PARTY_LICENSES.txt'), notices.join('\n\n' + '-'.repeat(72) + '\n\n') + '\n')
console.log(`editor built in ${out}`)
