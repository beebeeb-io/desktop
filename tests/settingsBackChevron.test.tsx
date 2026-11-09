/**
 * Settings > General: Back to Drive draws a left chevron, not a rotated right one.
 * The glyph is missing until `chevron-left` is in the shared icon paths.
 */
import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderToStaticMarkup } from 'react-dom/server'
import { Icon, type IconName } from '@beebeeb/shared/design'

test('chevron-left draws a left-pointing path', () => {
  const html = renderToStaticMarkup(<Icon name={'chevron-left' as IconName} size={14} />)
  expect(html).toContain('M15 18l-6-6 6-6')
})

test('the settings back link names chevron-left and does not rotate it', () => {
  const src = readFileSync(new URL('../src/v2/sections/settings/shell/SettingsView.tsx', import.meta.url), 'utf8')
  const at = src.indexOf('>Back to Drive<')
  const back = src.slice(Math.max(0, at - 400), at + 180)
  expect(back).toContain('name="chevron-left"')
  expect(back).not.toContain('rotate-180')
  expect(back).toContain('size={14}')
  expect(back).toContain('esc')
})
