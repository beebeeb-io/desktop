// P1b gate: the dev-only strings never ship. Walks a build output directory and counts the three strings the dev
// server uses (the fixture host key, the state seeds, the live state result attribute). Usage:
//   node scripts/check-v2-dev-strings.mjs [dir]      (default: dist)
//   node scripts/check-v2-dev-strings.mjs --self-test
// Exits 1 when a string is found, listing `file: string × count`; 0 otherwise.
import { mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'

/** The dev-only strings: in a built app each is 0 occurrences. */
export const DEV_STRINGS = ['__bbV2DevHost', '__bbV2Seed', 'data-v2-state-result']

/** What a build output carries. Source maps (`*.map`) and declarations (`*.d.ts`) are never counted. */
const BUILT = /\.(js|mjs|css|html)$/

/** Walk `dir` for files matching `pattern`; source maps and type declarations are skipped. */
function walk(dir, pattern, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, pattern, out)
    else if (pattern.test(name) && !name.endsWith('.map') && !name.endsWith('.d.ts')) out.push(p)
  }
  return out
}

/** Non-overlapping occurrences of `needle` in `haystack`. */
function count(haystack, needle) {
  let n = 0
  for (let at = haystack.indexOf(needle); at !== -1; at = haystack.indexOf(needle, at + needle.length)) n += 1
  return n
}

/**
 * Count each dev string and each file holding one. `files` is the number of files scanned, so a clean build prints a
 * non-zero file count next to `dev-strings=0` (a scan of nothing cannot pass). `pattern` widens the extension filter for
 * the test's source-tree positive control; the CLI keeps the build output's extensions.
 */
export function scan(dir, pattern = BUILT) {
  const files = walk(dir, pattern)
  const hits = []
  let devStrings = 0
  for (const p of files) {
    const text = readFileSync(p, 'utf8')
    const file = relative(dir, p)
    for (const string of DEV_STRINGS) {
      const n = count(text, string)
      if (n > 0) {
        hits.push({ file, string, count: n })
        devStrings += n
      }
    }
  }
  return { files: files.length, devStrings, hits }
}

/**
 * Prove the scanner rejects a planted file and accepts a clean one, under two temp dirs. Exits 1 on either mismatch.
 */
function selfTest() {
  const planted = mkdtempSync(join(tmpdir(), 'bb-v2-dev-strings-planted-'))
  const clean = mkdtempSync(join(tmpdir(), 'bb-v2-dev-strings-clean-'))
  try {
    mkdirSync(join(planted, 'assets'))
    writeFileSync(join(planted, 'assets', 'app.js'), "globalThis['__bbV2Seed'] = {}\n")
    writeFileSync(join(clean, 'app.js'), 'export const answer = 42\n')
    const bad = scan(planted)
    const good = scan(clean)
    const rejected = bad.devStrings === 1 && bad.hits.length === 1 && bad.hits[0].string === '__bbV2Seed'
    const accepted = good.devStrings === 0 && good.hits.length === 0 && good.files > 0
    if (!rejected || !accepted) {
      console.error(`self-test failed: planted dev-strings=${bad.devStrings} clean dev-strings=${good.devStrings}`)
      process.exitCode = 1
      return
    }
    console.log('self-test=ok')
  } finally {
    rmSync(planted, { recursive: true, force: true })
    rmSync(clean, { recursive: true, force: true })
  }
}

if (import.meta.main) {
  const arg = process.argv[2]
  if (arg === '--self-test') {
    selfTest()
  } else {
    const dir = arg ?? 'dist'
    const result = scan(dir)
    if (result.devStrings > 0) {
      for (const hit of result.hits) console.error(`${hit.file}: ${hit.string} × ${hit.count}`)
      console.error(`files=${result.files} dev-strings=${result.devStrings}`)
      process.exitCode = 1
    } else {
      console.log(`files=${result.files} dev-strings=${result.devStrings}`)
    }
  }
}
