/**
 * Read a `const NAME: &str = "…";` from the Rust source, so a frontend test can hand the exact
 * sentence Lane R returns to the surface under test and pin that it is shown verbatim. Reading it
 * here (instead of copying the text into the test) keeps the two from drifting: a Rust rewording
 * changes what the test feeds in, and the test still holds the frontend to "verbatim".
 *
 * Only plain string literals are supported (no `concat!`, no raw strings); anything else throws, so a
 * constant that changes shape fails loudly instead of being read wrong.
 */
import { readFileSync } from 'node:fs'

export function rustSource(file: string): string {
  return readFileSync(new URL(`../../src-tauri/src/${file}`, import.meta.url), 'utf8')
}

export function rustStr(file: string, name: string): string {
  const source = rustSource(file)
  const pattern = new RegExp(`(?:pub(?:\\([a-z]+\\))? )?const ${name}: &str =\\s*"((?:[^"\\\\]|\\\\.)*)";`, 'g')
  const matches = [...source.matchAll(pattern)]
  if (matches.length !== 1) throw new Error(`expected one plain string constant ${name} in ${file}, found ${matches.length}`)
  return matches[0][1].replace(/\\(["\\n])/g, (_, c: string) => (c === 'n' ? '\n' : c))
}
