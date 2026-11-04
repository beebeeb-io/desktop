import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

/**
 * Spec §4.8 (LD-0057): the Rust files that `src-tauri/src/lib.rs` declares as `#[cfg(test)] mod x;`
 * (or `#[cfg(all(test, …))]`). Inline, those modules were `#[cfg(test)]` items of `lib.rs`, which a
 * scanner left out; as files of their own they are left out the same way.
 */
export function rustTestFiles(repo: string): Set<string> {
  const dir = join(repo, 'src-tauri/src')
  const files = new Set<string>()
  const lib = join(dir, 'lib.rs')
  if (!existsSync(lib)) return files
  const lines = readFileSync(lib, 'utf8').replace(/\r\n/g, '\n').split('\n')
  lines.forEach((line, at) => {
    const declaration = /^(?:pub )?mod ([a-z_][a-z0-9_]*);$/.exec(line)
    if (!declaration) return
    let testOnly = false
    let path: string | null = null
    for (let k = at - 1; k >= 0 && lines[k]!.startsWith('#['); k--) {
      if (lines[k]!.startsWith('#[cfg(test)]') || lines[k]!.startsWith('#[cfg(all(test')) testOnly = true
      const value = /^#\[path = "([^"]+)"\]$/.exec(lines[k]!)
      if (value) path = value[1]!
    }
    if (!testOnly) return
    const name = declaration[1]!
    for (const file of path ? [join(dir, path)] : [join(dir, `${name}.rs`), join(dir, name, 'mod.rs')]) {
      if (existsSync(file)) files.add(file)
    }
  })
  return files
}
