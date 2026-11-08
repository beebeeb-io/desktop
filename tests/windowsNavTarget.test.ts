// Task 2409: Home's "Not sent N" and the tray's waiting, held and given-up lines open Activity at its "Not sent"
// section. The tray sends the target as a string over the navigate event and localStorage; the main window parses it.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { NOT_DOWNLOADED_TARGET, NOT_SENT_TARGET, navIdFromString, navTargetFromString } from '../src/WindowsApp/nav'

describe('a navigation target', () => {
  test('a route, or Activity › Not sent; anything else is nothing', () => {
    expect(navTargetFromString(NOT_SENT_TARGET)).toEqual({ nav: 'activity', section: 'not-sent' })
    // Task 2414.
    expect(navTargetFromString(NOT_DOWNLOADED_TARGET)).toEqual({ nav: 'activity', section: 'not-downloaded' })
    expect(NOT_DOWNLOADED_TARGET).toBe('activity#not-downloaded')
    expect(navTargetFromString('activity')).toEqual({ nav: 'activity', section: null })
    expect(navTargetFromString('trash')).toEqual({ nav: 'trash', section: null })
    for (const bad of [null, '', 'activity#timeline', 'home#not-sent', 'nowhere']) expect(navTargetFromString(bad)).toBeNull()
    expect(navIdFromString(NOT_SENT_TARGET)).toBeNull()
  })

  test('the tray sends the same target the main window reads', () => {
    const tray = readFileSync(new URL('../src/WindowsTray.tsx', import.meta.url), 'utf8')
    // The constants are imported, not copied, so the two cannot drift (2409 self-review).
    expect(tray).toContain("import { NOT_DOWNLOADED_TARGET, NOT_SENT_TARGET, WINDOWS_APP_NAV_EVENT, WINDOWS_APP_NAV_STORAGE_KEY } from './WindowsApp/nav'")
    expect(tray).not.toContain("const NOT_SENT_TARGET =")
    expect(tray).not.toContain("const NOT_DOWNLOADED_TARGET =")
    expect(tray).toContain('await emit(WINDOWS_APP_NAV_EVENT, target)')
    expect(tray).toContain('window.localStorage.setItem(WINDOWS_APP_NAV_STORAGE_KEY, target)')
    expect(tray).toContain("{ target: NOT_SENT_TARGET, href: '#not-sent', title: 'See which changes' }")
    expect(tray).toContain("{ target: NOT_DOWNLOADED_TARGET, href: '#not-downloaded', title: 'See which files' }")
    // Both status lines go through the link-or-text helper.
    expect(tray).toContain('{statusLine(activity.headline)}')
    expect(tray).toContain('{statusLine(line)}')
  })
})
