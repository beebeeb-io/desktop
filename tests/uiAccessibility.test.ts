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
    // Task 1683 slice 5: the tone is now theme tokens (no oklch literals); the success variant
    // is still the green ok-* pair, never an amber token. Contrast: tests/toastContrast.test.tsx.
    const tone = toastToneForVariant('success')
    expect(tone).toEqual({
      background: 'var(--ok-bg)',
      border: 'var(--ok-line)',
      iconBackground: 'var(--ok-icon)',
      iconGlyph: 'var(--paper)',
      title: 'var(--ink)',
      message: 'var(--ink-2)',
      close: 'var(--ink-3)',
    })
    expect(Object.values(tone).join(' ')).not.toContain('amber')
  })
})
