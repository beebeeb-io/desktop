// The window's Content Security Policy (src-tauri/tauri*.conf.json), pinned on two points (lead ruling on M3d's fork 5,
// 2026-11-09): decrypted audio and video play from the window's own blob: URLs (`media-src 'self' blob:`, exactly), and
// no decrypted content is ever framed (no `blob:` in `frame-src`: PDFs move to pdf.js). Audio and video run no script,
// and only the window's own scripts (`script-src 'self'`) can mint a blob: URL.
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

const TAURI = join(import.meta.dir, '..', 'src-tauri')

/** A CSP's directives: name → sources. Tauri takes the policy as one string or as an object of directives. */
export function directives(csp: string | Record<string, string | string[]>): Map<string, string[]> {
  const entries = typeof csp === 'string'
    ? csp.split(';').map((d) => d.trim().split(/\s+/)).filter((d) => d[0]).map(([name, ...sources]) => [name, sources] as const)
    : Object.entries(csp).map(([name, v]) => [name, Array.isArray(v) ? v : v.split(/\s+/).filter(Boolean)] as const)
  return new Map(entries.map(([name, sources]) => [name.toLowerCase(), sources]))
}

const configs = readdirSync(TAURI)
  .filter((f) => /^tauri(\.[\w-]+)?\.conf\.json$/.test(f))
  .map((file) => ({ file, csp: JSON.parse(readFileSync(join(TAURI, file), 'utf8'))?.app?.security?.csp }))
  .filter((c) => c.csp !== undefined && c.csp !== null)

describe('the window’s CSP (M3d fork 5)', () => {
  test('the base config sets it', () => {
    expect(configs.map((c) => c.file)).toContain('tauri.conf.json')
  })
  test('nothing decrypted is framed: frame-src is set and has no blob:, and neither has default-src', () => {
    for (const { file, csp } of configs) {
      const d = directives(csp)
      expect({ file, frameSrc: d.has('frame-src') }).toEqual({ file, frameSrc: true })
      expect({ file, frameBlob: d.get('frame-src')!.includes('blob:') }).toEqual({ file, frameBlob: false })
      expect({ file, defaultBlob: (d.get('default-src') ?? []).includes('blob:') }).toEqual({ file, defaultBlob: false })
    }
  })
  test('media plays from the window’s own blob: URLs: media-src is exactly \'self\' blob:', () => {
    for (const { file, csp } of configs) {
      expect({ file, mediaSrc: directives(csp).get('media-src') }).toEqual({ file, mediaSrc: ["'self'", 'blob:'] })
    }
  })
})
