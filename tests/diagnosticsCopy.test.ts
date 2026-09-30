/**
 * Task 1685: the support bundle used to promise "without secrets or plaintext
 * names" while the Rust export still carried file paths and names, and a toast
 * said the bundle was "written to your sync folder" while nothing was written.
 *
 * These tests pin the copy to what the export now guarantees and keep the old
 * claims from drifting back in anywhere under `src/`.
 */
import { describe, expect, it } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import {
  SUPPORT_BUNDLE_DETAIL,
  SUPPORT_BUNDLE_SAVED_TITLE,
  supportBundleSavedMessage,
} from '../src/diagnosticsCopy'

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) return sourceFiles(path)
    return /\.(ts|tsx)$/.test(entry) ? [path] : []
  })
}

const SRC = join(import.meta.dir, '..', 'src')
const FILES = sourceFiles(SRC)

describe('support bundle copy (task 1685)', () => {
  it('scans a real source tree', () => {
    // A scan over zero files would pass vacuously.
    expect(FILES.length).toBeGreaterThan(20)
    expect(FILES.some((f) => f.endsWith('pages/Account.tsx'))).toBe(true)
  })

  it('no source file still makes the old blanket claims', () => {
    const banned = [
      /without secrets or plaintext names/i,
      /plaintext names/i,
      /written to your sync folder/i,
      /bundle is being written/i,
    ]
    const hits = FILES.flatMap((file) => {
      const text = readFileSync(file, 'utf8')
      return banned.filter((re) => re.test(text)).map((re) => `${file}: ${re}`)
    })
    expect(hits).toEqual([])
  })

  it('states exactly what the export keeps and removes', () => {
    expect(SUPPORT_BUNDLE_DETAIL).toContain('queue counts')
    expect(SUPPORT_BUNDLE_DETAIL).toContain('error code')
    expect(SUPPORT_BUNDLE_DETAIL).toContain('Paths, file and folder names and sign-in tokens are removed')
    // Everything the export keeps is named: ids and the server address too, and the residual.
    expect(SUPPORT_BUNDLE_DETAIL).toContain('Standard error words, numbers, opaque ids and the server address are kept')
    expect(SUPPORT_BUNDLE_DETAIL).toContain('a name the app does not recognise can stay only if it is itself a standard word or a number')
    expect(SUPPORT_BUNDLE_DETAIL).not.toContain('only standard error words and numbers are kept')
    // Brand voice: no reassurance words.
    expect(SUPPORT_BUNDLE_DETAIL).not.toMatch(/\b(secure|safe|bank-grade|anonymi[sz]ed|100%)\b/i)
  })

  it('the saved toast names the file path and says the user attaches it', () => {
    const message = supportBundleSavedMessage('/tmp/beebeeb-diagnostics-1.json')
    expect(SUPPORT_BUNDLE_SAVED_TITLE).toBe('Support bundle saved')
    expect(message).toContain('/tmp/beebeeb-diagnostics-1.json')
    expect(message).toContain('attach the file yourself')
  })

  it('a failed email draft is reported as a saved bundle, not a failed save', () => {
    const failed = supportBundleSavedMessage('/tmp/b.json', false)
    expect(failed).toContain('/tmp/b.json')
    expect(failed).toContain('send the file to support@beebeeb.io yourself')
    expect(failed).not.toContain('is opening')
    const account = readFileSync(join(SRC, 'pages', 'Account.tsx'), 'utf8')
    expect(account).toContain('result.value.email_opened')
    expect(account).toContain("'warning'")
  })

  it('the Account button calls report_problem (which writes the bundle), not export_diagnostics', () => {
    const account = readFileSync(join(SRC, 'pages', 'Account.tsx'), 'utf8')
    expect(account).toContain("'report_problem'")
    expect(account).not.toContain("'export_diagnostics'")
  })
})
