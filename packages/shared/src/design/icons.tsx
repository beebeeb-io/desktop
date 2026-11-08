// The shared icon set and IconName union.
import type { SVGProps } from 'react'

export type IconName =
  | 'shield'
  | 'lock'
  | 'key'
  | 'eye'
  | 'eye-off'
  | 'check'
  | 'chevron-right'
  | 'chevron-down'
  | 'upload'
  | 'folder'
  | 'file'
  | 'file-text'
  | 'file-spreadsheet'
  | 'file-presentation'
  | 'file-code'
  | 'file-archive'
  | 'file-audio'
  | 'file-video'
  | 'file-data'
  | 'users'
  | 'search'
  | 'plus'
  | 'star'
  | 'trash'
  | 'settings'
  | 'share'
  | 'cloud'
  | 'link'
  | 'clock'
  | 'download'
  | 'copy'
  | 'mail'
  | 'bell'
  | 'x'
  | 'image'
  | 'more'
  | 'arrow-up'
  | 'pause'
  | 'play'
  | 'menu'
  | 'info'
  | 'activity'
  | 'camera'
  | 'scan'
  | 'pin'
  | 'flag'
  | 'edit'
  | 'columns'
  | 'globe'
  | 'chat'

interface IconProps extends SVGProps<SVGSVGElement> {
  name: IconName
  size?: number
}

const paths: Record<IconName, string> = {
  shield:
    'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z',
  lock:
    'M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2zM7 11V7a5 5 0 0 1 10 0v4',
  key:
    'M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4',
  eye:
    'M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  'eye-off':
    'M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24M1 1l22 22',
  check: 'M20 6L9 17l-5-5',
  'chevron-right': 'M9 18l6-6-6-6',
  'chevron-down': 'M6 9l6 6 6-6',
  upload:
    'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12',
  folder:
    'M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z',
  file:
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7',
  'file-text':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M16 13H8M16 17H8M10 9H8',
  'file-spreadsheet':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M8 13h2v2H8zM14 13h2v2h-2zM8 17h2v2H8zM14 17h2v2h-2z',
  'file-presentation':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M8 13h8M8 17h5',
  'file-code':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 13l-2 2 2 2M14 13l2 2-2 2',
  'file-archive':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 2v2M12 4v2M10 6v2M12 8v2M10 10v2M12 12v4h-2v-4',
  'file-audio':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M11 15a2 2 0 1 1-4 0 2 2 0 0 1 4 0zM11 15v-4l4-1',
  'file-video':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 15l5-3-5-3v6z',
  'file-data':
    'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M8 17v-2M12 17v-4M16 17v-3',
  users:
    'M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75',
  search:
    'M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35',
  plus: 'M12 5v14M5 12h14',
  star:
    'M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z',
  trash:
    'M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2',
  settings:
    'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z',
  share:
    'M18 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.59 13.51l6.83 3.98M15.41 6.51l-6.82 3.98',
  cloud:
    'M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z',
  link:
    'M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71',
  clock:
    'M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2',
  download:
    'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3',
  copy:
    'M20 9h-9a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-9a2 2 0 0 0-2-2zM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1',
  mail:
    'M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2zM22 6l-10 7L2 6',
  bell:
    'M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M13.73 21a2 2 0 0 1-3.46 0',
  x: 'M18 6L6 18M6 6l12 12',
  image:
    'M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21',
  more: 'M12 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM19 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM5 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2z',
  'arrow-up': 'M12 19V5M5 12l7-7 7 7',
  pause: 'M6 4h4v16H6zM14 4h4v16h-4z',
  play: 'M5 3l14 9-14 9V3z',
  menu: 'M3 12h18M3 6h18M3 18h18',
  // Info: circle with an "i" (dot tittle + short body line). The dot uses the
  // standard zero-length-stroke trick so it renders as a round cap.
  info: 'M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01',
  activity: 'M22 12h-4l-3 9L9 3l-3 9H2',
  // Camera: classic camera body with lens circle
  camera:
    'M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2zM12 17a4 4 0 1 0 0-8 4 4 0 0 0 0 8z',
  // Scan: document scan / corner brackets
  scan:
    'M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2M8 12h8M12 8v8',
  // Pin: Lucide-style pushpin — angled pin with shaft line to corner
  pin:
    'M12 17v5M8 17h8M12 3v5M5 8h14a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z',
  // Flag: Lucide-style pennant on a pole — used by the admin "Feature flags"
  // nav item and the pool integrity-issue indicator.
  flag: 'M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1zM4 22v-7',
  // Edit: Lucide-style pencil — for the text-editor "Edit" affordance (task 1563)
  edit: 'M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z M15 5l4 4',
  // Columns: two vertical panes — for the editor's markdown split view (task 1563)
  columns: 'M4 4h16v16H4z M12 4v16',
  // Globe and Chat: the VPN and Chat product glyphs (task 2322). The globe, not the shield, stands for
  // VPN: the shield is reserved for encryption state.
  globe:
    'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9M12 3c-2.5 2.6-3.8 5.6-3.8 9s1.3 6.4 3.8 9',
  chat:
    'M21 11.5a8.4 8.4 0 0 1-12.3 7.4L3 21l2.1-5.6A8.4 8.4 0 1 1 21 11.5z',
}

/** The raw 24-grid path of a glyph, for components that draw it inside another SVG (the product icons). */
export function iconPath(name: IconName): string {
  return paths[name]
}

export function Icon({ name, size = 24, ...rest }: IconProps) {
  // Look up via a widened type so an unknown name (e.g. one passed as a plain
  // string from a nav/data config that bypasses the IconName union) yields
  // `undefined` instead of a typed `string`.
  const d = (paths as Record<string, string | undefined>)[name]

  // A missing glyph is cosmetic — it must NEVER crash the whole app. Render a
  // sized, empty svg (preserves layout) rather than throwing on
  // `paths[name].split(...)`. Caused a full-page ErrorBoundary crash on the
  // admin dashboard 2026-05-29 when the 'flag' icon name had no path.
  if (d == null) {
    if ((import.meta as { env?: { DEV?: boolean } }).env?.DEV) {
      console.warn(`[Icon] unknown icon name "${String(name)}" — add it to icons.tsx`)
    }
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true" {...rest} />
    )
  }

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...rest}
    >
      {/* One <path> for every subpath: splitting at `z` cut a relative `m` (the key's) loose from the
          point it is relative to, so it started at the origin and drew a stray line. */}
      <path d={d} />
    </svg>
  )
}
