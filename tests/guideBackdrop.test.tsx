/**
 * The first-run guide dims the window and lifts its card. The tray tip keeps its own look.
 */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { installMiniDom, type MiniDomInstall } from './fixtures/miniDom'

let dom: MiniDomInstall
beforeAll(() => { dom = installMiniDom('') })
afterAll(() => dom.restore())

test('a guide tip lays a neutral scrim behind the card', async () => {
  const { GuideTip } = await import('../src/v2/guide/Guide')
  const html = renderToStaticMarkup(<GuideTip index={0} onNext={() => {}} onSkip={() => {}} />)
  const scrimAt = html.indexOf('bg-ink/35')
  const cardAt = html.indexOf('data-guide-card')
  expect(scrimAt).toBeGreaterThanOrEqual(0)
  expect(cardAt).toBeGreaterThan(scrimAt)
  expect(html).toContain('pointer-events-none')
  expect(html).not.toContain('bg-blue')
  expect(html).not.toContain('bg-amber')
})

test('the guide card is a light surface on a larger radius, and the tray tip has no scrim', async () => {
  const { GuideTip, TrayTipCard } = await import('../src/v2/guide/Guide')
  const html = renderToStaticMarkup(<GuideTip index={0} onNext={() => {}} onSkip={() => {}} />)
  const cardAt = html.indexOf('data-guide-card')
  const card = html.slice(Math.max(0, cardAt - 240), cardAt + 500)
  expect(card).toContain('rounded-xl')
  expect(card).toContain('bg-paper')
  expect(card).toContain('shadow-xl')
  expect(card).toContain('border-line')
  expect(card).toContain('text-ink')
  const tray = renderToStaticMarkup(<TrayTipCard onGotIt={() => {}} />)
  expect(tray).toContain('data-tray-tip')
  expect(tray).not.toContain('bg-ink/35')
})
