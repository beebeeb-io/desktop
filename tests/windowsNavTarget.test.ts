// Task 2409: Home's "Not sent N" and the tray's waiting, held and given-up lines open Activity at its "Not sent"
// section. The tray sends the target as a string over the navigate event and localStorage; the main window parses it.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { NOT_SENT_TARGET, navIdFromString, navTargetFromString } from '../src/WindowsApp/nav'

describe('a navigation target', () => {
  test('a route, or Activity › Not sent; anything else is nothing', () => {
    expect(navTargetFromString(NOT_SENT_TARGET)).toEqual({ nav: 'activity', section: 'not-sent' })
    expect(navTargetFromString('activity')).toEqual({ nav: 'activity', section: null })
    expect(navTargetFromString('trash')).toEqual({ nav: 'trash', section: null })
    for (const bad of [null, '', 'activity#timeline', 'home#not-sent', 'nowhere']) expect(navTargetFromString(bad)).toBeNull()
    expect(navIdFromString(NOT_SENT_TARGET)).toBeNull()
  })

  test('the tray sends the same target the main window reads', () => {
    const tray = readFileSync(new URL('../src/WindowsTray.tsx', import.meta.url), 'utf8')
    expect(tray).toContain(`const NOT_SENT_TARGET = '${NOT_SENT_TARGET}'`)
    expect(tray).toContain('await emit(WINDOWS_APP_NAV_EVENT, NOT_SENT_TARGET)')
    expect(tray).toContain('window.localStorage.setItem(WINDOWS_APP_NAV_STORAGE_KEY, NOT_SENT_TARGET)')
    // Both status lines go through the link-or-text helper.
    expect(tray).toContain('{statusLine(activity.headline)}')
    expect(tray).toContain('{statusLine(line)}')
  })
})
