import { describe, expect, test } from 'bun:test'
import { modalFocusTargetIndex, toastToneForVariant } from '../src/windows/ui'

describe('windows ui accessibility helpers', () => {
  test('cycles modal focus forward and backward within the focusable set', () => {
    expect(modalFocusTargetIndex(2, 3, 'forward')).toBe(0)
    expect(modalFocusTargetIndex(0, 3, 'backward')).toBe(2)
    expect(modalFocusTargetIndex(1, 3, 'forward')).toBe(2)
    expect(modalFocusTargetIndex(1, 3, 'backward')).toBe(0)
  })

  test('keeps generic success toasts off the amber brand accent', () => {
    expect(toastToneForVariant('success')).toEqual({
      background: 'oklch(0.96 0.04 155)',
      border: 'oklch(0.84 0.08 155)',
      color: 'oklch(0.28 0.09 155)',
      iconBackground: 'oklch(0.44 0.11 155)',
    })
  })
})
