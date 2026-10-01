/**
 * Parts of the two tray surfaces that are identical, shared by the Windows flyout
 * (`WindowsTray.tsx`) and the macOS menu-bar popover (`macPopover/MacPopover.tsx`)
 * (task 1683 slice 3, spec `docs/specs/2026-09-30-macos-menubar-popover.md` section 11).
 *
 * What is shared, and what is not:
 *  - SHARED: the file-type classification, the 28 pt header icon button and the footer action
 *    button. The Windows flyout renders them with their DEFAULT props, so its DOM, geometry
 *    and colours are exactly what they were before the extraction. The regression guard is a
 *    DOM + bounding-box + computed-colour snapshot of `?window=tray` taken before and after
 *    (`tests/render-windows-tray-snapshot.mjs`, evidence under `/home/user/evidence/1683-s3/`).
 *  - NOT shared, on purpose: layout, type scale, copy and glyphs. The spec says the macOS shell
 *    and the Windows shell "keep their own layout, sizes and copy ("Recycle bin", "Explorer"
 *    stay Windows-only)". The Windows `FILE_COLOR` hex values and its hard-coded `BrandMark`
 *    stay in `WindowsTray.tsx`; neither is to be copied (spec section 11).
 *
 * The buttons take every metric that differs between the shells as a prop whose default is the
 * Windows value, so a caller that passes nothing gets the Windows flyout's look.
 */
import type { CSSProperties, MouseEvent, ReactNode } from 'react'

export type TrayFileType = 'pdf' | 'image' | 'video' | 'audio' | 'code' | 'archive' | 'default'

/** Ported from repos/web file-icon.tsx `getFileType` (pure, no @beebeeb/shared import). */
export function getFileType(name: string): TrayFileType {
  const ext = name.split('.').pop()?.toLowerCase() ?? ''

  if (ext === 'pdf') return 'pdf'

  if (
    ['jpg', 'jpeg', 'png', 'gif', 'heic', 'heif', 'webp', 'svg', 'ico', 'bmp', 'tiff', 'tif', 'avif', 'dng', 'cr2', 'cr3', 'nef', 'arw', 'orf', 'rw2', 'raf'].includes(ext)
  )
    return 'image'
  if (['mp4', 'mov', 'avi', 'mkv', 'webm', 'flv', 'wmv', 'm4v', 'hevc'].includes(ext)) return 'video'
  if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'wma', 'm4a', 'aiff', 'opus'].includes(ext)) return 'audio'

  if (
    ['js', 'jsx', 'ts', 'tsx', 'py', 'rs', 'go', 'rb', 'java', 'kt', 'swift', 'c', 'cpp', 'h', 'hpp', 'cs', 'php', 'sh', 'bash', 'zsh', 'lua', 'r', 'scala', 'zig', 'asm', 'sql', 'graphql', 'proto'].includes(ext)
  )
    return 'code'
  if (['html', 'htm', 'css', 'scss', 'sass', 'less', 'vue', 'svelte', 'astro'].includes(ext)) return 'code'

  if (['zip', 'tar', 'gz', 'rar', '7z', 'bz2', 'xz', 'zst', 'lz', 'dmg', 'iso'].includes(ext)) return 'archive'

  return 'default'
}

/** The 28 x 28 header icon button (the gear). */
export function TrayIconButton({
  title,
  ariaLabel,
  onClick,
  children,
  hoverBackground = 'var(--paper-3)',
  className,
  aria,
  buttonRef,
  style: extraStyle,
}: {
  title?: string
  ariaLabel: string
  onClick: (event: MouseEvent<HTMLButtonElement>) => void
  children: ReactNode
  hoverBackground?: string
  className?: string
  /** Extra aria attributes (`aria-haspopup`, ...), applied as given. */
  aria?: Record<string, string | boolean>
  buttonRef?: (el: HTMLButtonElement | null) => void
  /** Merged over the defaults (the macOS popover pulls the glyph edge onto its gutter). */
  style?: CSSProperties
}) {
  return (
    <button
      ref={buttonRef}
      title={title}
      aria-label={ariaLabel}
      onClick={onClick}
      className={className}
      {...aria}
      style={{
        width: 28,
        height: 28,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: 'var(--ink-3)',
        background: 'transparent',
        border: 'none',
        borderRadius: 6,
        cursor: 'pointer',
        ...extraStyle,
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = hoverBackground
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = 'transparent'
      }}
    >
      {children}
    </button>
  )
}

/** One footer action: icon over label. Defaults are the Windows flyout's. */
export function TrayActionButton({
  icon,
  label,
  onClick,
  borderLeft,
  disabled = false,
  gap = 5,
  padding = '10px 4px',
  fontSize = 11,
  fontWeight = 500,
  hoverBackground = 'var(--paper-3)',
  disabledOpacity = 0.5,
  className,
  title,
  style: extraStyle,
}: {
  icon: ReactNode
  label: ReactNode
  onClick: () => void
  borderLeft?: boolean
  disabled?: boolean
  gap?: number
  padding?: string
  fontSize?: number
  fontWeight?: number
  hoverBackground?: string
  disabledOpacity?: number
  className?: string
  title?: string
  style?: CSSProperties
}) {
  const style: CSSProperties = {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    gap,
    padding,
    fontSize,
    fontFamily: 'var(--font-sans)',
    fontWeight,
    color: 'var(--ink-2)',
    background: 'transparent',
    border: 'none',
    borderLeft: borderLeft ? '1px solid var(--line)' : 'none',
    cursor: disabled ? 'default' : 'pointer',
    opacity: disabled ? disabledOpacity : 1,
    lineHeight: 1,
    ...extraStyle,
  }
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={className}
      title={title}
      style={style}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = hoverBackground
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = 'transparent'
      }}
    >
      {icon}
      {label}
    </button>
  )
}
