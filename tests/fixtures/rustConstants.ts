/**
 * Read a `const NAME: &str = "…";` from the Rust source, so a frontend test can hand the exact
 * sentence Lane R returns to the surface under test and pin that it is shown verbatim. Reading it
 * here (instead of copying the text into the test) keeps the two from drifting: a Rust rewording
 * changes what the test feeds in, and the test still holds the frontend to "verbatim".
 *
 * Only plain string literals are supported (no `concat!`, no raw strings); anything else throws, so a
 * constant that changes shape fails loudly instead of being read wrong.
 */
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const RUST_SRC = fileURLToPath(new URL('../../src-tauri/src/', import.meta.url))

export function rustSource(file: string): string {
  if (file === 'lib.rs') return libRsSource()
  // A module the engine crate took over (`account_binding.rs`, `state_db/...`) is read there.
  const shell = new URL(`../../src-tauri/src/${file}`, import.meta.url)
  return readFileSync(existsSync(shell) ? shell : new URL(`../../crates/beebeeb-engine/src/${file}`, import.meta.url), 'utf8')
}

/**
 * `lib.rs` after its split (LD-0057 PR 2): each `mod <child>;` line whose file opens with
 * `` //! `<child>` items split out of `lib.rs`. `` is replaced by that child's items (its first three
 * lines, the header, a blank line and `use super::*;`, were never part of `lib.rs`), so the crate root
 * reads back in its old order. The Rust twin is `crate::root_source::lib_rs`. `dir` is a `src-tauri/src`
 * (a scratch copy of the tree, for the tests that mutate one).
 */
export function libRsSource(dir: string = RUST_SRC): string {
  const hub = readFileSync(join(dir, 'lib.rs'), 'utf8')
  let children = 0
  const out = hub
    .split(/(?<=\n)/)
    .map((line) => {
      const text = childOf(dir, line)
      if (text === null) return line
      children += 1
      // The split's only edit inside a child is a `pub(super) ` on a private item, field or method. As in the Rust
      // twin: the first three lines go, then one blank line if it follows them.
      const body = text
        .split(/(?<=\n)/)
        .slice(3)
        .map((l) => l.replace(/^(\s*)pub\(super\) /, '$1'))
        .join('')
      return body.replace(/^\r?\n/, '')
    })
    .join('')
  checkChildren(children, dir)
  return out
}

/** How many children the split moved out of `lib.rs` (`CHILDREN` in `root_source.rs`). */
const CHILDREN = 19

/** A child whose header stopped matching drops its lines from every scanner's input: count them. 0 is a tree that was
 * never split (a scratch fixture). */
function checkChildren(found: number, dir: string): void {
  if (found !== 0 && found !== CHILDREN) throw new Error(`${dir}/lib.rs splices ${CHILDREN} children back in, found ${found}`)
}

/** The text of the child the hub line `line` (`mod <child>;`) declares, when it is one split out of `lib.rs`. */
function childOf(dir: string, line: string): string | null {
  const child = /^mod ([a-z_][a-z0-9_]*);\r?\n?$/.exec(line)?.[1]
  const path = child === undefined ? '' : join(dir, `${child}.rs`)
  if (child === undefined || !existsSync(path)) return null
  const text = readFileSync(path, 'utf8')
  return text.startsWith(`//! \`${child}\` items split out of \`lib.rs\`.`) ? text : null
}

/**
 * The files (names under `dir`) that `libRsSource` splices back into `lib.rs`. A scanner that reads the tree
 * file by file reads `lib.rs` as `libRsSource` and skips these, so it reads each line once, where it read it before.
 */
export function libRsChildren(dir: string = RUST_SRC): string[] {
  const lib = join(dir, 'lib.rs')
  if (!existsSync(lib)) return []
  const children = readFileSync(lib, 'utf8')
    .split(/(?<=\n)/)
    .filter((line) => childOf(dir, line) !== null)
    .map((line) => `${/^mod ([a-z_][a-z0-9_]*);/.exec(line)![1]}.rs`)
  checkChildren(children.length, dir)
  return children
}

/**
 * A module after a split: the facade `<module>.rs` (when it still exists) plus every `.rs` beside it
 * under `<module>/`, so a constant or function that moved to a child is still found. Test children
 * (`tests/`, `tests.rs`, `*_tests.rs`) are left out, and each file is cut at its own
 * `#[cfg(test)] mod tests` block, so a pin reads production code only.
 */
export function rustModuleSource(module: string): string {
  const files: string[] = []
  const facade = join(RUST_SRC, `${module}.rs`)
  if (existsSync(facade)) files.push(facade)
  const dir = join(RUST_SRC, module)
  if (existsSync(dir)) collect(dir, files)
  return files.map((file) => withoutTestModule(readFileSync(file, 'utf8'))).join('\n')
}

function collect(dir: string, out: string[]): void {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) {
      if (name !== 'tests') collect(path, out)
    } else if (name.endsWith('.rs') && name !== 'tests.rs' && !name.endsWith('_tests.rs')) {
      out.push(path)
    }
  }
}

function withoutTestModule(source: string): string {
  const start = source.indexOf('\n#[cfg(test)]\nmod tests {')
  return start === -1 ? source : source.slice(0, start)
}

function plainConstant(source: string, name: string, where: string): string {
  const pattern = new RegExp(`(?:pub(?:\\([a-z]+\\))? )?const ${name}: &str =\\s*"((?:[^"\\\\]|\\\\.)*)";`, 'g')
  const matches = [...source.matchAll(pattern)]
  if (matches.length !== 1) throw new Error(`expected one plain string constant ${name} in ${where}, found ${matches.length}`)
  return matches[0][1].replace(/\\(["\\n])/g, (_, c: string) => (c === 'n' ? '\n' : c))
}

export function rustStr(file: string, name: string): string {
  return plainConstant(rustSource(file), name, file)
}

/** `rustStr` over a whole module after a split: the facade and every child, wherever the constant moved. */
export function rustStrIn(module: string, name: string): string {
  return plainConstant(rustModuleSource(module), name, module)
}
