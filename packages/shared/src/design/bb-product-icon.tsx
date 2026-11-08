// BBProductIcon: the glyph for each Beebeeb product name (task 2322).
import { iconPath } from './icons'
import type { IconName } from './icons'

// The glyph each product wears on its badge. The shield is deliberately absent: it is reserved for
// encryption state, so VPN carries the globe.
const PRODUCT_GLYPHS = {
  drive: 'folder',
  pass: 'key',
  vpn: 'globe',
  mail: 'mail',
  chat: 'chat',
} as const satisfies Record<string, IconName>

export type BBProduct = keyof typeof PRODUCT_GLYPHS

export const BB_PRODUCT_NAMES: Record<BBProduct, string> = {
  drive: 'Drive',
  pass: 'Pass',
  vpn: 'VPN',
  mail: 'Mail',
  chat: 'Chat',
}

export type BBProductTone = 'light' | 'dark'

// Fixed brand colours, not theme tokens: the tokens switch with the page theme, while the mark picks its
// tone with the `tone` prop (as repos/core/brand/logo-mark.svg and logo-mark-dark.svg do).
const AMBER = '#F5B800'
const INK = '#1A1714'
const TONES: Record<BBProductTone, { tile: string; ink: string }> = {
  light: { tile: AMBER, ink: INK },
  dark: { tile: INK, ink: AMBER },
}

// The geometry below is repos/core/brand/products/build.py (option A, task 2322); change both together.
const TILE_SIZE = 512
const TILE_RADIUS = 102
// The Inter Bold "b" of repos/core/brand/logo-mark.svg as a path, so it renders without the font.
const B_GLYPH_PATH =
  'M754 -19Q663 -19 600.0 12.0Q537 43 497.5 88.5Q458 134 437 179H423V0H128V1490H428V930H437Q457 974 495.5 1021.0Q534 1068 597.5 1100.0Q661 1132 756 1132Q880 1132 983.0 1068.0Q1086 1004 1147.5 876.0Q1209 748 1209 557Q1209 371 1149.0 242.5Q1089 114 986.0 47.5Q883 -19 754 -19ZM662 222Q779 222 840.5 316.0Q902 410 902 558Q902 705 841.0 798.5Q780 892 662 892Q546 892 483.5 801.0Q421 710 421 558Q421 406 484.0 314.0Q547 222 662 222Z'
const B_SCALE = 0.1723
const B_ORIGIN = { x: 116.8, y: 360.7 }
const BADGE_CENTER = 378
const BADGE_RING_RADIUS = 104
const BADGE_DISC_RADIUS = 88
const GLYPH_SIZE = 104
const GLYPH_GRID = 24
const GLYPH_SCALE = GLYPH_SIZE / GLYPH_GRID
const GLYPH_ORIGIN = BADGE_CENTER - GLYPH_SIZE / 2
const GLYPH_STROKE = 13 / GLYPH_SCALE

interface BBProductIconProps {
  product: BBProduct
  /** Edge length in px (default 40). */
  size?: number
  tone?: BBProductTone
  /** Accessible name. Without it the icon is decorative and hidden from assistive technology. */
  title?: string
}

export function BBProductIcon({ product, size = 40, tone = 'light', title }: BBProductIconProps) {
  const { tile, ink } = TONES[tone]
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${TILE_SIZE} ${TILE_SIZE}`}
      fill="none"
      role={title ? 'img' : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      style={{ flex: 'none' }}
    >
      {title && <title>{title}</title>}
      <rect width={TILE_SIZE} height={TILE_SIZE} rx={TILE_RADIUS} fill={tile} />
      <g fill={ink} transform={`translate(${B_ORIGIN.x} ${B_ORIGIN.y}) scale(${B_SCALE} ${-B_SCALE})`}>
        <path d={B_GLYPH_PATH} />
      </g>
      <circle cx={BADGE_CENTER} cy={BADGE_CENTER} r={BADGE_RING_RADIUS} fill={tile} />
      <circle cx={BADGE_CENTER} cy={BADGE_CENTER} r={BADGE_DISC_RADIUS} fill={ink} />
      <g
        transform={`translate(${GLYPH_ORIGIN} ${GLYPH_ORIGIN}) scale(${Number(GLYPH_SCALE.toFixed(4))})`}
        stroke={tile}
        strokeWidth={Number(GLYPH_STROKE.toFixed(2))}
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d={iconPath(PRODUCT_GLYPHS[product])} />
      </g>
    </svg>
  )
}
