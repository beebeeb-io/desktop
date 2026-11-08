// BBProductLockup: the product glyph locked up with the Beebeeb wordmark (task 2322).
import { BB_PRODUCT_NAMES, BBProductIcon } from './bb-product-icon'
import type { BBProduct, BBProductTone } from './bb-product-icon'

// The proportions of the approved full-logo render (task 2322, option 3), all as fractions of the icon:
// icon 52 px, gap 14, wordmark 34, label 12, label gap 4.
const REFERENCE_ICON_SIZE = 52
const ICON_TO_TEXT_GAP = 14 / REFERENCE_ICON_SIZE
const WORDMARK_SIZE = 34 / REFERENCE_ICON_SIZE
const LABEL_SIZE = 12 / REFERENCE_ICON_SIZE
const LABEL_GAP = 4 / REFERENCE_ICON_SIZE
// Below this the mono label stops being legible, so it stops scaling with the icon.
const MIN_LABEL_SIZE = 9
// The wordmark as repos/core/brand/logo-full.svg draws it: letter-spacing -2 at font-size 72.
const WORDMARK_TRACKING_EM = -2 / 72
const LABEL_TRACKING_EM = 0.18

interface BBProductLockupProps {
  product: BBProduct
  /** Icon edge length in px (default 40); the wordmark and the label scale with it. */
  size?: number
  tone?: BBProductTone
}

export function BBProductLockup({ product, size = 40, tone = 'light' }: BBProductLockupProps) {
  return (
    <span className="inline-flex items-center" style={{ gap: size * ICON_TO_TEXT_GAP }}>
      {/* A dark tile on a dark surface needs a hairline to keep its edge; the radius is the tile's 102/512. */}
      <span
        className="inline-flex flex-none"
        style={{
          borderRadius: '20%',
          boxShadow: tone === 'dark' ? '0 0 0 1px var(--color-line-2)' : undefined,
        }}
      >
        <BBProductIcon product={product} size={size} tone={tone} />
      </span>
      {/* Not BBLogo: it tracks at -0.025em, the logo-full.svg wordmark at -2/72 em. */}
      <span className="inline-flex flex-col" style={{ gap: size * LABEL_GAP }}>
        <span
          className="whitespace-nowrap font-bold leading-none text-ink"
          style={{ fontSize: size * WORDMARK_SIZE, letterSpacing: `${WORDMARK_TRACKING_EM}em` }}
        >
          beebeeb<span className="text-amber">.io</span>
        </span>
        <span
          className="font-mono font-medium uppercase text-ink-2"
          style={{
            fontSize: Math.max(size * LABEL_SIZE, MIN_LABEL_SIZE),
            letterSpacing: `${LABEL_TRACKING_EM}em`,
            lineHeight: 'normal',
          }}
        >
          {BB_PRODUCT_NAMES[product]}
        </span>
      </span>
    </span>
  )
}
