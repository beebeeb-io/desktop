import { describe, expect, test } from 'bun:test'
import { unknownFilesSignOutCopy } from '../src/signOutUnknownFilesCopy'

describe('the Windows sign-out choice for files from before the last update (task 2191)', () => {
  test('it says how many, where the copies go, and that they are not encrypted', () => {
    const copy = unknownFilesSignOutCopy(3, 'C:\\Users\\a\\Beebeeb kept files 2026-10-26 1200')
    expect(copy.body).toStartWith('3 files in your Beebeeb folder were on this PC')
    expect(copy.keepNote).toContain('C:\\Users\\a\\Beebeeb kept files 2026-10-26 1200')
    expect(copy.keepNote).toContain('aren’t encrypted')
    expect(copy.deleteNote).toContain('We can’t recover it')
  })

  test('one file reads in the singular', () => {
    const copy = unknownFilesSignOutCopy(1, 'D:\\kept')
    expect(copy.body).toStartWith('1 file in your Beebeeb folder was on this PC')
    expect(copy.body).toContain('whether it holds')
  })
})
