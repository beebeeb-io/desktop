import { describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

const dir = path.join(import.meta.dir, '..', '..', '.github', 'workflows')
const text = fs.readFileSync(path.join(dir, 'release-native-macos.yml'), 'utf8')
const windowsText = fs.readFileSync(path.join(dir, 'release-native-windows.yml'), 'utf8')
const wf = (Bun as any).YAML.parse(text)
const concurrencyGroup = (t: string) => /group:\s*(desktop-native-publish-manifest)\b/.exec(t)?.[1]

type Step = { name?: string; run?: string; uses?: string; env?: Record<string, string> }
const jobs = Object.values(wf.jobs) as { steps: Step[] }[]
const allSteps: Step[] = jobs.flatMap((j) => j.steps)

describe('release-native-macos workflow', () => {
  test('never builds macOS: no cargo, xcodebuild, a macOS runner, or a build script', () => {
    expect(text).not.toMatch(/runs-on:\s*macos/)
    for (const step of allSteps) {
      expect(step.run ?? '').not.toMatch(/cargo (build|rustc)|xcodebuild|bundle-app\.sh|sign-app\.sh|notarise\.sh|codesign/)
    }
  })

  test('the signing job names the desktop-release-macos environment, not the Windows one', () => {
    expect(wf.jobs['publish-manifest'].environment).toBe('desktop-release-macos')
    expect(text).not.toMatch(/environment:\s*desktop-release\s*$/m)
  })

  test('secrets appear only in env: blocks, never in run: or with: text', () => {
    for (const step of allSteps) {
      expect(step.run ?? '').not.toContain('secrets.')
      expect(step.run ?? '').not.toContain('${{')
    }
    const signing = allSteps.filter((s) => s.env && Object.values(s.env).some((v) => v.includes('secrets.UPDATER_V2_PRIVATE_KEY')))
    expect(signing.length).toBe(2)
    for (const line of text.split('\n')) {
      if (line.includes('secrets.') && !line.includes('secrets.GITHUB_TOKEN')) {
        // every secret reference is an `NAME: ${{ secrets.X }}` env line or the releases checkout token
        expect(line).toMatch(/^\s+(token|[A-Z0-9_]+): \$\{\{ secrets\.[A-Z0-9_]+ \}\}\s*$/)
      }
    }
  })

  test('the blockers gate is a prerequisite of the publishing job', () => {
    const gate = wf.jobs.gate.steps.map((s: Step) => s.run ?? '').join('\n')
    expect(gate).toContain('release-blockers-check.sh macos')
    expect(wf.jobs['publish-manifest'].needs).toBe('gate')
    // nothing before the gate in the publish job writes to the host
    const publishAt = text.indexOf('publish-release-files.sh stage')
    expect(text.indexOf('release-blockers-check.sh macos')).toBeLessThan(publishAt)
  })

  test('shares the concurrency group with the Windows workflow, one at a time', () => {
    expect(wf.jobs['publish-manifest'].concurrency.group).toBe('desktop-native-publish-manifest')
    expect(wf.jobs['publish-manifest'].concurrency['cancel-in-progress']).toBe(false)
    expect(concurrencyGroup(windowsText)).toBe('desktop-native-publish-manifest')
  })

  test('verifies the tarball before the manifest is written and keeps existing entries', () => {
    const verifyAt = text.indexOf('--verify "../work/mac/$name"')
    const writeAt = text.indexOf('--mac-artifact')
    expect(verifyAt).toBeGreaterThan(0)
    expect(verifyAt).toBeLessThan(writeAt)
    expect(text).toContain('--existing')
  })
})
