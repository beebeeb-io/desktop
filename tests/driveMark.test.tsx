// The sidebar's brand row shows the drive mark and, under it in the same row, the words "Beebeeb Drive"
// above the Files link (board MacSide, .mk-side-brand). The window caption names the product too.
import { describe, expect, test } from 'bun:test'
import { BBProductLockup } from '@beebeeb/shared/design'
import { Sidebar } from '../src/v2/chrome/Sidebar'
import { windowTitle } from '../src/v2/V2App'
import { elementsOf } from './fixtures/componentHarness'

const sidebar = () => Sidebar({ active: 'files', onGo: () => {}, storage: null, accountName: 'Anna' })

describe('the sidebar shows Beebeeb Drive', () => {
  test('the brand row carries the drive lockup at 24 px and the words Beebeeb Drive above Files', () => {
    const els = elementsOf(sidebar())
    const lockup = els.find((el) => el.type === BBProductLockup)
    expect(lockup?.props.product).toBe('drive')
    expect(lockup?.props.size).toBe(24)
    const words = els.findIndex((el) => el.props.children === 'Beebeeb Drive')
    const files = els.findIndex((el) => el.props.item?.section === 'files')
    expect(words).toBeGreaterThanOrEqual(0)
    expect(files).toBeGreaterThan(words)
  })
})

describe('the window caption names Beebeeb Drive', () => {
  test('Files is Beebeeb Drive', () => {
    expect(windowTitle('files')).toBe('Beebeeb Drive')
  })

  test('Settings is Settings - Beebeeb Drive', () => {
    expect(windowTitle('settings')).toBe('Settings - Beebeeb Drive')
  })
})
