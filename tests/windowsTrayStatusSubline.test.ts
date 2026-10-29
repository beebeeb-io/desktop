// The Windows tray's line under each recent file (spec 1888/2226 D13): a file that is still uploading is "Not uploaded
// yet", never "Synced to" its folder. `statusSubline` is not exported, so the harness runs its real declaration.
import { describe, expect, test } from 'bun:test'
import { loadComponent } from './fixtures/componentHarness'

const statusSubline = loadComponent('WindowsTray.tsx', 'statusSubline', {}) as (status: string, folder: string) => string

describe('the tray’s line under a recent file', () => {
  test('an uploading row is never shown as synced', () => {
    expect(statusSubline('uploading', 'Screenshots')).toBe('Not uploaded yet — Screenshots')
    expect(statusSubline('uploading', 'Screenshots')).not.toMatch(/synced/i)
  })
})
