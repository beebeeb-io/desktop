// P1b gate: the dev-only strings must be in 0 built files. This scanner walks a directory and counts the three
// strings the dev server and the live state route use (`__bbV2DevHost`, `__bbV2Seed`, `data-v2-state-result`).
// A string in a comment inside a built JS file still counts: it ships. Source maps and `.d.ts` are not built code.
//
//   node scripts/check-v2-dev-strings.mjs [dir]   # default: dist
//   node scripts/check-v2-dev-strings.mjs --self-test
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

/** The dev-only strings the P1b gate names. */
export const DEV_STRINGS = ['__bbV2DevHost', '__bbV2Seed', 'data-v2-state-result']

const SCANNED = /\.(?:js|mjs|cjs|css|html|ts|tsx)$/
const isScanned = (name) => SCANNED.test(name) && !name.endsWith('.d.ts') && !name.endsWith('.map')

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (isScanned(name)) out.push(p)
  }
  return out
}

/** Walk `dir` and count the dev-only strings. `files` is every file scanned; `devStrings` every occurrence. */
export function scan(dir) {
  const files = walk(dir)
  const hits = []
  let devStrings = 0
  for (const file of files) {
    const body = readFileSync(file, 'utf8')
    for (const string of DEV_STRINGS) {
      const count = body.split(string).length - 1
      if (count > 0) {
        hits.push({ file, string, count })
        devStrings += count
      }
    }
  }
  return { files: files.length, devStrings, hits }
}

function selfTest() {
  const root = mkdtempSync(join(tmpdir(), 'bb-v2-dev-strings-'))
  const planted = join(root, 'planted')
  const clean = join(root, 'clean')
  try {
    for (const d of [planted, clean]) {
      mkdirSync(d)
      writeFileSync(join(d, 'app.js'), 'const x = 1\n')
    }
    writeFileSync(join(planted, 'app.js'), 'const k = "__bbV2Seed"\n')
    const bad = scan(planted)
    const good = scan(clean)
    if (bad.devStrings !== 1 || good.devStrings !== 0) {
      console.error(`self-test failed: planted=${bad.devStrings} (want 1) clean=${good.devStrings} (want 0)`)
      process.exitCode = 1
      return
    }
    console.log('self-test=ok')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

function main(argv) {
  if (argv.includes('--self-test')) return selfTest()
  const dir = argv.find((a) => !a.startsWith('--')) ?? 'dist'
  const result = scan(dir)
  console.log(`files=${result.files} dev-strings=${result.devStrings}`)
  if (result.devStrings > 0) {
    for (const hit of result.hits) console.log(`${hit.file}: ${hit.string} × ${hit.count}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main(process.argv.slice(2))
