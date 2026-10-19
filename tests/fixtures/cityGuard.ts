/**
 * Task 2050 (Guus, 2026-10-19): a storage location is a COUNTRY, never a city. Cities together
 * identify the hosting provider (docs/canon/data-residency.md), so no user-facing string and no code
 * path of the desktop may name one or read one off the wire.
 *
 * What the guard scans: every `.ts` / `.tsx` under `src/` and every `.rs` under `src-tauri/src/`,
 * with comments removed, and for Rust with every `#[cfg(test)]` item removed (test code may name a
 * city to prove it is absent). Two rules:
 *   1. a city NAME (Falkenstein, Helsinki, Nuremberg, Nürnberg, Ede as a place) appears nowhere;
 *   2. no code reads a `city` / `example_city` field: that is how a city reaches the screen from
 *      the payload (the server sends `city` for one more release and it must stay unread).
 * Fixtures under `tests/` and `src-tauri/tests/` are outside the scan: they prove absence.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

export const CITY_NAME = /falkenstein|helsinki|nuremberg|n(?:ü|ue|u)rnberg/i
const ED_E_PLACE = /\bEde\b/
/** A `city` field read, written or declared: `.city`, `city:`, `"city"`, `example_city`, `city,` in a destructure. */
export const CITY_FIELD = /\bexample_city\b|\.city\b|\bcity\s*[:?]|["'`]city["'`]|\{[^}]*\bcity\b[^}]*\}|\bpub city\b/

export function stripComments(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, match => match.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:\\'"`])\/\/.*$/gm, '$1')
}

/**
 * Blank the braces inside Rust comments, string literals (plain, raw, byte) and char literals, so
 * brace matching sees only code braces. A tokenizer, not a regex: `'"'` and lifetimes (`'a`) must not
 * confuse the string state, or the rest of a 20,000-line file is mis-balanced.
 */
function maskLiterals(src: string): string {
  const out = src.split('')
  const blank = (from: number, to: number) => {
    for (let k = from; k < to; k++) if (out[k] === '{' || out[k] === '}') out[k] = ' '
  }
  let i = 0
  while (i < src.length) {
    const c = src[i]
    const next = src[i + 1]
    if (c === '/' && next === '/') {
      const end = src.indexOf('\n', i)
      const stop = end === -1 ? src.length : end
      blank(i, stop)
      i = stop
    } else if (c === '/' && next === '*') {
      let depth = 1
      let k = i + 2
      while (k < src.length && depth > 0) {
        if (src[k] === '/' && src[k + 1] === '*') { depth++; k += 2 }
        else if (src[k] === '*' && src[k + 1] === '/') { depth--; k += 2 }
        else k++
      }
      blank(i, k)
      i = k
    } else if ((c === 'r' || (c === 'b' && next === 'r')) && /^b?r#*"/.test(src.slice(i, i + 12))) {
      const open = /^b?r(#*)"/.exec(src.slice(i, i + 12))!
      const close = `"${open[1]}`
      const end = src.indexOf(close, i + open[0].length)
      const stop = end === -1 ? src.length : end + close.length
      blank(i, stop)
      i = stop
    } else if (c === '"') {
      let k = i + 1
      while (k < src.length && src[k] !== '"') k += src[k] === '\\' ? 2 : 1
      blank(i, k + 1)
      i = k + 1
    } else if (c === "'") {
      if (next === '\\') {
        const end = src.indexOf("'", i + 3)
        const stop = end === -1 ? src.length : end + 1
        blank(i, stop)
        i = stop
      } else if (src[i + 2] === "'") {
        blank(i, i + 3)
        i += 3
      } else {
        i++ // a lifetime
      }
    } else {
      i++
    }
  }
  return out.join('')
}

/** Remove every test-only (`#[cfg(test)]`, `#[cfg(all(test, ..))]`) item (a `mod tests { ... }`, a test fn, a `use`) from Rust source. */
export function stripRustTestItems(src: string): string {
  const masked = maskLiterals(src)
  const out: string[] = []
  // `cfg(test)` and `cfg(all(test, ...))` are test-only; `cfg(any(test, ...))` also builds in production and is scanned.
  const attr = /#\[cfg\((?:test|all\(test,[^\n]*)\)\]/g
  let cursor = 0
  for (let m = attr.exec(masked); m; m = attr.exec(masked)) {
    if (m.index < cursor) continue
    out.push(src.slice(cursor, m.index))
    // Skip to the item's end: the first `;` at depth 0, or the matching `}` of its first `{`.
    let i = m.index + m[0].length
    let depth = 0
    let opened = false
    for (; i < masked.length; i++) {
      const c = masked[i]
      if (c === '{') { depth++; opened = true }
      else if (c === '}') { depth--; if (opened && depth === 0) { i++; break } }
      else if (c === ';' && !opened) { i++; break }
    }
    out.push(src.slice(m.index, i).replace(/[^\n]/g, ' ')) // keep the line numbers
    cursor = i
    attr.lastIndex = i
  }
  out.push(src.slice(cursor))
  return out.join('')
}

export function hits(source: string, kind: 'ts' | 'rs'): string[] {
  const code = stripComments(kind === 'rs' ? stripRustTestItems(source) : source)
  const found: string[] = []
  code.split('\n').forEach((line, i) => {
    const name = line.match(CITY_NAME)
    if (name || ED_E_PLACE.test(line)) found.push(`${i + 1}: city name: ${line.trim().slice(0, 120)}`)
    else if (CITY_FIELD.test(line)) found.push(`${i + 1}: city field: ${line.trim().slice(0, 120)}`)
  })
  return found
}

function walk(dir: string, ext: RegExp, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, ext, out)
    else if (ext.test(name)) out.push(p)
  }
  return out
}

/** Every scanned file under `root` with its hits (files with none are left out). */
export function scan(root: string): { files: number; offenders: Record<string, string[]> } {
  const ts = walk(join(root, 'src'), /\.(ts|tsx)$/)
  const rs = walk(join(root, 'src-tauri/src'), /\.rs$/)
  const offenders: Record<string, string[]> = {}
  for (const file of [...ts, ...rs]) {
    const found = hits(readFileSync(file, 'utf8'), file.endsWith('.rs') ? 'rs' : 'ts')
    if (found.length > 0) offenders[relative(root, file)] = found
  }
  return { files: ts.length + rs.length, offenders }
}
