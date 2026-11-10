// P1b base item 6: the dev server serves the same isolation headers the Tauri main window carries. One source of
// truth would be enough if it were shared, but the dev server (vite.config.ts) and the window
// (src-tauri/tauri.conf.json) are two files: these tests read both, so a later edit to one side fails here.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import config from '../vite.config'

const COOP = 'Cross-Origin-Opener-Policy'
const COEP = 'Cross-Origin-Embedder-Policy'
const EXPECTED = { [COOP]: 'same-origin', [COEP]: 'require-corp' }

const conf = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8')) as {
  app: { windows: { label: string }[]; security: { headers: Record<string, string> } }
}

describe('the vite dev server sends the isolation headers (P1b)', () => {
  test('server.headers is exactly COOP same-origin + COEP require-corp', () => {
    expect(config.server?.headers).toEqual(EXPECTED)
  })

  test('the dev headers equal the main window’s headers in src-tauri/tauri.conf.json', () => {
    expect(conf.app.windows.map((w) => w.label)).toContain('main-app')
    const windowHeaders = conf.app.security.headers
    expect(config.server?.headers?.[COOP]).toBe(windowHeaders[COOP])
    expect(config.server?.headers?.[COEP]).toBe(windowHeaders[COEP])
  })

  test('server.headers is a plain object, so a static read of the config sees it', () => {
    const headers = config.server?.headers
    expect(typeof headers).toBe('object')
    expect(typeof headers).not.toBe('function')
    expect(Object.getPrototypeOf(headers)).toBe(Object.prototype)
    expect(JSON.parse(JSON.stringify(headers))).toEqual(EXPECTED)
  })
})
