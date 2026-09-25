/**
 * Inline (not `<img src="...">`-loaded) rendering of `assets/logo-full.svg`,
 * so its fills can track the live theme via the document's CSS custom
 * properties (`--ink`, `--amber`, `--amber-ink`).
 *
 * `assets/logo-full.svg` bakes its "beebeeb" wordmark to a literal hex
 * (`#1A1714`, the light-theme ink color) — fine for an `<img>` in light mode,
 * but an `<img src="*.svg">` is an opaque reference: the SVG document it
 * loads never inherits the host page's CSS custom properties, so that fill
 * cannot react to `data-theme="dark"`. In dark mode the wordmark rendered
 * permanently dark-on-dark, leaving only the amber ".io" visible (task
 * 1523). Rendering the same markup inline in this document's own DOM lets
 * `fill="var(--ink)"` resolve against the live theme, matching how the mark
 * itself is theme-INVARIANT (amber square + fixed-dark "b", per
 * `repos/core/brand/README.md`) while the wordmark tracks ink/paper like
 * `BBLogo` does on web (`repos/web/src/components/bb-logo.tsx`).
 */
export function Wordmark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 800 100" className={className} role="img" aria-label="beebeeb.io">
      {/* Mark — amber square, fixed-dark "b" (never inverts with theme). */}
      <rect x="0" y="6" width="88" height="88" rx="18" fill="var(--amber)" />
      <text
        x="44"
        y="70"
        textAnchor="middle"
        fontFamily="Inter, system-ui, -apple-system, sans-serif"
        fontSize="60"
        fontWeight="800"
        fill="var(--amber-ink)"
        letterSpacing="-2"
      >
        b
      </text>
      {/* Wordmark — "beebeeb" tracks ink (dark in light mode, light in dark
          mode), ".io" stays amber (theme-constant). */}
      <text
        x="108"
        y="76"
        fontFamily="Inter, system-ui, -apple-system, sans-serif"
        fontSize="72"
        fontWeight="700"
        letterSpacing="-2"
      >
        <tspan fill="var(--ink)">beebeeb</tspan>
        <tspan fill="var(--amber)">.io</tspan>
      </text>
    </svg>
  )
}
