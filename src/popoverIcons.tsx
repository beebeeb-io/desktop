/**
 * The popover's icons (task 1683 slice 3), drawn exactly as the two-surface hi-fi's symbol sheet
 * (`design/hifi/macos-two-surface.html`, `<symbol id="i-…">`): 20×20, stroke only, `currentColor`.
 */
import type { ReactElement } from 'react'

const PATHS: Readonly<Record<string, ReactElement>> = {
  gear: (
    <>
      <circle cx="10" cy="10" r="2.6" />
      <path d="M10 1.9v2.3M10 15.8v2.3M1.9 10h2.3M15.8 10h2.3M4.3 4.3l1.6 1.6M14.1 14.1l1.6 1.6M4.3 15.7l1.6-1.6M14.1 5.9l1.6-1.6" />
      <circle cx="10" cy="10" r="5.6" />
    </>
  ),
  folder: <path d="M2.5 6.2A1.7 1.7 0 0 1 4.2 4.5h3.3l1.7 1.9h6.6a1.7 1.7 0 0 1 1.7 1.7v6.7a1.7 1.7 0 0 1-1.7 1.7H4.2a1.7 1.7 0 0 1-1.7-1.7z" />,
  globe: (
    <>
      <circle cx="10" cy="10" r="7.5" />
      <path d="M2.5 10h15M10 2.5c2.2 2.3 3.2 4.8 3.2 7.5s-1 5.2-3.2 7.5c-2.2-2.3-3.2-4.8-3.2-7.5s1-5.2 3.2-7.5z" />
    </>
  ),
  db: (
    <>
      <ellipse cx="10" cy="5" rx="6" ry="2.4" />
      <path d="M4 5v10c0 1.3 2.7 2.4 6 2.4s6-1.1 6-2.4V5M4 10c0 1.3 2.7 2.4 6 2.4s6-1.1 6-2.4" />
    </>
  ),
  lock: (
    <>
      <rect x="4" y="9" width="12" height="8.5" rx="2" />
      <path d="M6.8 9V6.6a3.2 3.2 0 0 1 6.4 0V9" />
    </>
  ),
  unlock: (
    <>
      <rect x="4" y="9" width="12" height="8.5" rx="2" />
      <path d="M6.8 9V6.6a3.2 3.2 0 0 1 6.2-1.1" />
    </>
  ),
  pause: <path d="M7.5 5v10M12.5 5v10" />,
  play: <path d="M7 5l8 5-8 5z" />,
  refresh: <path d="M16 9.5A6 6 0 0 0 5.3 6.1M4 10.5a6 6 0 0 0 10.7 3.4M5 3v3.4h3.4M15 17v-3.4h-3.4" />,
  alert: (
    <>
      <circle cx="10" cy="10" r="7.5" />
      <path d="M10 6v4.6M10 13.6v.1" />
    </>
  ),
  check: <path d="M5 10.4l3.2 3.1L15 6.7" />,
  up: <path d="M10 15.5v-11M5.5 9L10 4.5 14.5 9" />,
  down: <path d="M10 4.5v11M5.5 11l4.5 4.5 4.5-4.5" />,
  file: (
    <>
      <path d="M5 2.5h6.5L15.5 6.5v10a1 1 0 0 1-1 1h-9.5a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1z" />
      <path d="M11.5 2.5v4h4" />
    </>
  ),
  image: (
    <>
      <rect x="3" y="4" width="14" height="12" rx="1.6" />
      <circle cx="7.5" cy="8.3" r="1.4" />
      <path d="M3.5 14.5l4-4 3 3 2-2 4 4" />
    </>
  ),
  sync: <path d="M15.5 8a6 6 0 0 0-10.6-1.6M4.5 12a6 6 0 0 0 10.6 1.6M4.5 3.5v3h3M15.5 16.5v-3h-3" />,
}

export function PopoverIcon({ name, size = 16, strokeWidth = 1.6, label }: { name: string; size?: number; strokeWidth?: number; label?: string }) {
  return (
    <svg
      className="pop-i"
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {PATHS[name] ?? null}
    </svg>
  )
}
