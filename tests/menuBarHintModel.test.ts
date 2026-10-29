/**
 * The words and the rule of Home's one-time hint (task 1683 slice 6; spec `docs/specs/2026-10-24-macos-two-surface-app.md`
 * §7.6; hi-fi section 3). Pure, so every sentence and every state is pinned without a window.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { MENU_BAR_HINT_BODY, MENU_BAR_HINT_DISMISS, MENU_BAR_HINT_TITLE, forgetHintDismissedThisRun, hintDismissedThisRun, menuBarHintState, rememberHintDismissed } from '../src/menuBarHintModel'

describe('the words', () => {
  test('are the spec\'s, byte for byte', () => {
    expect(MENU_BAR_HINT_TITLE).toBe('Beebeeb lives in the menu bar')
    expect(MENU_BAR_HINT_BODY).toBe('Click the b at the top of your screen to see what’s syncing. You can close this window; Beebeeb keeps running.')
    expect(MENU_BAR_HINT_DISMISS).toBe('Got it')
  })

  test('are the ones the hi-fi draws', () => {
    const html = readFileSync(new URL('../design/hifi/macos-two-surface.html', import.meta.url), 'utf8')
    expect(html).toContain(`<b>${MENU_BAR_HINT_TITLE}</b><span>${MENU_BAR_HINT_BODY}</span>`)
    expect(html).toContain(`<button class="btn">${MENU_BAR_HINT_DISMISS}</button>`)
  })

  test('are plain: no developer word, no provider, no emoji', () => {
    const all = `${MENU_BAR_HINT_TITLE} ${MENU_BAR_HINT_BODY} ${MENU_BAR_HINT_DISMISS}`
    expect(all).not.toMatch(/daemon|hydrat|engine|control cent|socket|heartbeat|hetzner/i)
    expect(all).not.toMatch(/\p{Extended_Pictographic}/u)
  })
})

describe('when it shows', () => {
  test('only when the Mac says it was not dismissed', () => {
    expect(menuBarHintState({ ok: true, value: false })).toBe('show')
    expect(menuBarHintState({ ok: true, value: true })).toBe('hide')
  })

  test('not before the Mac has answered, so it never flashes in and out', () => {
    expect(menuBarHintState(null)).toBe('unknown')
  })

  test('not when the answer could not be read: a tip is not worth an error on Home', () => {
    expect(menuBarHintState({ ok: false })).toBe('hide')
  })
})

describe('a dismissal is remembered for the run, whatever the save did', () => {
  test('a Mac that says it was not dismissed still shows nothing once the run has dismissed it', () => {
    expect(menuBarHintState({ ok: true, value: false }, true)).toBe('hide')
    expect(menuBarHintState(null, true)).toBe('hide')
    expect(menuBarHintState({ ok: false }, true)).toBe('hide')
  })

  test('the run\'s flag starts clear, is set by a dismissal, and is cleared for a test', () => {
    forgetHintDismissedThisRun()
    expect(hintDismissedThisRun()).toBe(false)
    rememberHintDismissed()
    expect(hintDismissedThisRun()).toBe(true)
    forgetHintDismissedThisRun()
    expect(hintDismissedThisRun()).toBe(false)
  })
})
