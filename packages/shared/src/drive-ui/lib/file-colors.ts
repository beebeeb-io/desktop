/**
 * Badge color styles for file type indicators in the file list.
 *
 * Returns an object with `color` and `background` CSS values so callers can
 * apply them via inline `style` — this keeps us from needing extra Tailwind
 * tokens for colors that are not part of the design system (purple, blue).
 */

import type { FileType } from '../file-icon'

export interface BadgeStyle {
  color: string
  background: string
}

// Matches the colorMap palette already used in file-icon.tsx.
const BADGE_STYLES: Record<FileType, BadgeStyle> = {
  folder:     { color: 'var(--color-ink-3)',    background: 'var(--color-paper-3)' },
  pdf:        { color: '#dc2626',               background: 'rgba(220,38,38,0.08)' },
  word:       { color: '#2563eb',               background: 'rgba(37,99,235,0.08)' },
  excel:      { color: '#16a34a',               background: 'rgba(22,163,74,0.08)' },
  powerpoint: { color: '#ea580c',               background: 'rgba(234,88,12,0.08)' },
  image:      { color: 'var(--color-green)',     background: 'var(--color-green-bg)' },
  video:      { color: '#9333ea',               background: 'rgba(147,51,234,0.08)' },
  audio:      { color: 'var(--color-amber-deep)', background: 'var(--color-amber-bg)' },
  code:       { color: '#0d9488',               background: 'rgba(13,148,136,0.08)' },
  archive:    { color: '#92400e',               background: 'rgba(146,64,14,0.08)' },
  data:       { color: 'var(--color-ink-3)',    background: 'var(--color-paper-3)' },
  md:         { color: '#0f766e',               background: 'rgba(15,118,110,0.08)' },
  fig:        { color: '#a855f7',               background: 'rgba(168,85,247,0.08)' },
  default:    { color: 'var(--color-ink-3)',    background: 'var(--color-paper-3)' },
}

export function getBadgeStyle(type: FileType): BadgeStyle {
  return BADGE_STYLES[type] ?? BADGE_STYLES.default
}
