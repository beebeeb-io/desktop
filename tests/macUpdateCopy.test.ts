/**
 * The words and the address of a Mac's update (task 2176, lead decision 2026-10-24 22:47; two-surface spec §10.2 #28, and
 * E27's amendment): the App Sandbox denies an in-place install, so an update on a Mac is DOWNLOADED, never installed. One
 * module holds them so every surface that offers an update (Settings › About, the popover's update row) says the same
 * thing in the same words. The bytes below are the popover lane's `src/macUpdateCopy.ts` too, so merging both is a rebase.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { MACOS_DOWNLOAD_URL, UPDATE_DOWNLOAD_HINT, UPDATE_DOWNLOAD_LABEL, updateAvailableLine } from '../src/macUpdateCopy'

describe('a Mac downloads its updates', () => {
  test('the words and the address are the ones the lead decided', () => {
    expect(MACOS_DOWNLOAD_URL).toBe('https://beebeeb.io/download/macos-dmg')
    expect(UPDATE_DOWNLOAD_LABEL).toBe('Download update')
    expect(UPDATE_DOWNLOAD_HINT).toBe('Drag Beebeeb into Applications to replace this version.')
    expect(updateAvailableLine('0.8.13')).toBe('Version 0.8.13 is available.')
  })

  test('nothing here offers a restart or an install', () => {
    const words = [MACOS_DOWNLOAD_URL, UPDATE_DOWNLOAD_LABEL, UPDATE_DOWNLOAD_HINT, updateAvailableLine('1.0.0')].join(' ')
    expect(words).not.toMatch(/restart|install|relaunch/i)
  })

  test('the module says why, so nobody adds the install back by reading only the strings', () => {
    const source = readFileSync(fileURLToPath(new URL('../src/macUpdateCopy.ts', import.meta.url)), 'utf8')
    expect(source).toContain('task 2176')
    expect(source).toContain('App Sandbox')
  })
})
