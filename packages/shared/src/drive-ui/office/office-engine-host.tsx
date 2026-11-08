/**
 * OfficeEngineHost (task 1567) — mounts the real LibreOffice-WASM engine in a
 * same-origin `<iframe>` and exposes a typed `OfficeBridge` handle once it's
 * ready.
 *
 * Why an iframe, not a direct script tag on the app's own page: the engine
 * needs `crossOriginIsolated === true` (SharedArrayBuffer for its pthread
 * build — see PLAN.md's COOP/COEP note) which is a document-level opt-in via
 * response headers. Scoping it to an iframe whose document is served from
 * `/office/<version>/...` (own headers, see nginx.conf + the dev-server
 * plugin in vite.config.ts) means the REST of the app never runs under
 * COOP/COEP, which would otherwise break any future cross-origin embed
 * elsewhere. The iframe is same-origin (the whole point of "our own engine,
 * our own origin" — docs/EGRESS.md), so `iframe.contentWindow.bbOffice` is
 * directly readable/callable from this component — no postMessage plumbing
 * needed, and every call in this file is a synchronous property read plus a
 * normal async function call on that object.
 *
 * `bb-office-api.js` (repos/office/bridge/) documents its own contract as
 * "load me as a sibling <script> on the same page as qt_soffice.html" — this
 * repo owns that combined page as `bb-office-host.html` (assembled by
 * scripts/office-dev-assets.sh from the engine's own qt_soffice.html plus a
 * trailing `<script src="bb-office-api.js">`), served from the SAME
 * content-hashed version directory as the engine binaries.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type { OfficeBridge } from '../lib/office/bb-office-bridge'
import { fetchOfficeManifest } from '../lib/office/loader'

export type OfficeEngineStatus = 'loading-manifest' | 'booting' | 'ready' | 'error'

export interface OfficeEngineHostProps {
  /** Base path the manifest + version directories are served under. */
  baseUrl?: string
  /** App theme at mount time — seeded into the engine profile BEFORE any
   *  document opens (see bridge's `setTheme` doc comment: switching theme
   *  live does not repaint an already-open document in this engine build,
   *  so this is a boot-time seed, not a reactive prop). */
  initialTheme: 'light' | 'dark'
  /** Fires once the engine is booted (`bbOffice` AND its `Module.uno_main`
   *  port exist, see `engineReadyBridge`) and the boot-time theme seed has
   *  been applied. */
  onReady?: (bridge: OfficeBridge) => void
  onError?: (message: string) => void
  /** Test-only: overrides the polling timeout (ms) so a red-path test doesn't
   *  wait the full production timeout. */
  bootTimeoutMs?: number
}

/**
 * How long the engine may take to boot, from the host document's `load` to
 * its UNO port existing. Task 1585 raised this from 90 s to 180 s together
 * with the readiness change below: a slow device (a phone, or a machine at
 * load 100+) legitimately needs longer than 90 s to instantiate the engine,
 * and "Preparing the editor on this device…" is the honest state until it
 * has, not an error.
 */
const DEFAULT_BOOT_TIMEOUT_MS = 180_000

type EngineWindow = Window & { bbOffice?: OfficeBridge; Module?: { uno_main?: unknown } }

/**
 * True once the engine can actually take a call: `bbOffice` exists (the
 * bridge script ran) AND the engine's `Module.uno_main` port exists (Qt's
 * loader has fetched and instantiated the engine). Exported for the unit test.
 *
 * Task 1585 item 4: this used to wait for `bbOffice` alone. That global
 * appears as soon as bb-office-api.js parses, long before the engine boots,
 * so `open()` then sat inside the bridge's OWN `waitForModule(60000)`. That
 * 60 s limit, not this host's budget, decided whether a slow boot failed (a
 * load-average-100 e2e run saw exactly that shape: no outline within 120 s).
 * Waiting for the port here means `open()` only ever runs against a booted
 * engine, and one budget (this one) governs the whole boot.
 */
export function engineReadyBridge(win: Window): OfficeBridge | null {
  try {
    const w = win as EngineWindow
    const bridge = w.bbOffice
    if (!bridge) return null
    return w.Module && w.Module.uno_main ? bridge : null
  } catch {
    // A navigated-away / cross-origin document: not ready (the caller's own
    // timeout reports it).
    return null
  }
}

/**
 * Polls for engine readiness rather than relying on the iframe's `onLoad` —
 * `onLoad` fires once the HTML document parses, well before Qt's own
 * `QtLoader` has fetched/instantiated the engine and exposed its
 * `Module.uno_main` port (see `engineReadyBridge`).
 */
function pollForBridge(
  win: Window,
  timeoutMs: number,
  signal: { cancelled: boolean },
): Promise<OfficeBridge> {
  const start = Date.now()
  return new Promise((resolve, reject) => {
    ;(function poll() {
      if (signal.cancelled) return
      const bridge = engineReadyBridge(win)
      if (bridge) {
        resolve(bridge)
        return
      }
      if (Date.now() - start > timeoutMs) {
        reject(new Error('Office engine did not become ready in time'))
        return
      }
      setTimeout(poll, 150)
    })()
  })
}

/** User-facing message when the engine document is not the one we serve. */
export const DAMAGED_HOST_MESSAGE = "The editor's files did not arrive intact on this device. Reload the page to try again."

/**
 * Returns an error message when the iframe's loaded document is not the
 * engine host page (task 1584), or null when it is, or when nothing can be
 * checked yet (the initial about:blank, or a document this realm may not
 * read). Exported for the unit test.
 */
export function inspectHostDocument(win: Window): string | null {
  let doc: Document | null = null
  try {
    if (win.location.href === 'about:blank') return null
    doc = win.document
  } catch {
    return null
  }
  if (!doc) return null
  const hasBridgeScript = Array.from(doc.getElementsByTagName('script')).some((el) => /(^|\/)bb-office-api\.js(\?|$)/.test(el.getAttribute('src') ?? ''))
  return hasBridgeScript ? null : DAMAGED_HOST_MESSAGE
}

/**
 * The engine bundle's license notices (task 1585 / 1567 open item): the MPL-2.0
 * §3.2(a) Source Code Form notice plus the Qt LGPL, Emscripten and OFL texts.
 * scripts/office-dev-assets.sh and the prod stager (office-bundle-stage.sh,
 * which refuses to stage without it) both place it in the version directory.
 */
export function officeNoticesUrl(baseUrl: string, version: string): string {
  return `${baseUrl}/${encodeURIComponent(version)}/THIRD_PARTY_NOTICES.txt`
}

/** Where the engine's complete source lives (named in the About panel). */
export const OFFICE_SOURCE_URL = 'https://github.com/beebeeb-io/office'

export function useOfficeEngine(opts: OfficeEngineHostProps) {
  const { baseUrl = '/office', initialTheme, onReady, onError, bootTimeoutMs = DEFAULT_BOOT_TIMEOUT_MS } = opts
  const [status, setStatus] = useState<OfficeEngineStatus>('loading-manifest')
  const [error, setError] = useState<string | null>(null)
  const [iframeSrc, setIframeSrc] = useState<string | null>(null)
  // The engine bundle's content-hashed version directory (task 1585: the
  // editor's About panel links that version's own THIRD_PARTY_NOTICES.txt).
  const [version, setVersion] = useState<string | null>(null)
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const bridgeRef = useRef<OfficeBridge | null>(null)
  const cancelledRef = useRef({ cancelled: false })

  useEffect(() => {
    cancelledRef.current = { cancelled: false }
    let cancelled = false
    setStatus('loading-manifest')
    setError(null)
    fetchOfficeManifest({ baseUrl })
      .then((manifest) => {
        if (cancelled) return
        setIframeSrc(`${baseUrl}/${manifest.version}/bb-office-host.html`)
        setVersion(manifest.version)
        setStatus('booting')
      })
      .catch((err) => {
        if (cancelled) return
        const message = err instanceof Error ? err.message : String(err)
        setError(message)
        setStatus('error')
        onError?.(message)
      })
    return () => {
      cancelled = true
      cancelledRef.current.cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseUrl])

  const handleIframeLoad = useCallback(() => {
    const win = iframeRef.current?.contentWindow
    if (!win) return
    // Task 1584: fail fast when the engine document itself arrived damaged.
    // On iPhone Safari, bb-office-host.html came in double-encoded (Brotli
    // + gzip, see nginx.conf); Safari undid one layer and rendered the rest
    // as text, and this hook would have polled for a bridge that can never
    // appear for the full 90 s boot timeout while the user watched symbols.
    // The real document always carries the bb-office-api.js <script> (it is
    // how `bbOffice` gets defined at all), so its absence after `load` means
    // this is not our document.
    const damage = inspectHostDocument(win)
    if (damage) {
      setError(damage)
      setStatus('error')
      onError?.(damage)
      return
    }
    const localSignal = { cancelled: false }
    cancelledRef.current = localSignal
    pollForBridge(win, bootTimeoutMs, localSignal)
      .then(async (bridge) => {
        if (localSignal.cancelled) return
        bridgeRef.current = bridge
        // Boot-time theme seed (lead decision 2 on the engine's live-repaint
        // gap): applied before any document is open, so there is nothing
        // wrong to un-repaint — the FIRST document this session opens gets
        // the seeded theme's icon/appearance config from the start.
        try {
          await bridge.setTheme(initialTheme)
        } catch {
          // Non-fatal — the engine's own dialogs simply keep their default
          // appearance; the document canvas itself is unaffected either way.
        }
        // bridge.setWorkspaceColor() (task 1567 fix pass item 1) is
        // deliberately NOT called here — it is applied by the CALLER, after
        // open()/newDocument() succeeds (see office-editor.tsx's
        // handleReady). Found empirically: Application::SetSettings()'s own
        // DataChangedEvent broadcast (vcl/source/app/svapp.cxx) only reaches
        // windows that EXIST at call time. Calling it at this point, before
        // any document/window exists (only the Start Center's own frame is
        // up), silently no-ops visually even though the call itself reports
        // `applied: true` — confirmed with a red (0xff0000) probe two ways:
        // called before newDocument() → no visible change; called after →
        // the whole canvas repaints red immediately, no extra invalidate
        // needed.
        if (localSignal.cancelled) return
        setStatus('ready')
        onReady?.(bridge)
      })
      .catch((err) => {
        if (localSignal.cancelled) return
        const message = err instanceof Error ? err.message : String(err)
        setError(message)
        setStatus('error')
        onError?.(message)
      })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bootTimeoutMs, initialTheme])

  const noticesUrl = version ? officeNoticesUrl(baseUrl, version) : null
  return { status, error, iframeSrc, iframeRef, handleIframeLoad, bridge: bridgeRef.current, version, noticesUrl }
}

export interface OfficeEngineHostViewProps extends OfficeEngineHostProps {
  className?: string
}

/**
 * Native Qt title-bar crop (lead decision 1 on the engine's open gap,
 * task 1567 phase-4 note: "the native Qt window title bar cannot be
 * hidden" — a frameless-window engine patch was tried, built, and reverted
 * after it broke document-switching; the lead's own fallback is "crop it in
 * our web shell... our chrome replaces LibreOffice's"). MEASURED, not
 * guessed: a real screenshot of the rebuilt engine's title bar
 * ("private:stream — LibreOfficeDev Writer", solid `rgb(48,140,198)`) was
 * scanned pixel-by-pixel — the coloured bar plus its 4px white window-edge
 * sliver above it spans the first ~22px of the iframe's own content, so
 * 26px is cropped for a safety margin without eating into the document
 * canvas below it. Mechanism: the iframe renders TALLER than its visible
 * container (`height: calc(100% + Npx)`) and is shifted up by the same
 * amount inside an `overflow: hidden` parent — the extra height at the
 * BOTTOM reveals that much more real document canvas instead of wasting it,
 * and `getBoundingClientRect()` on the (now-shifted) iframe element already
 * reflects its true on-screen position, so office-editor.tsx's own
 * pointerup → host-coordinate translation for the floating selection
 * toolbar needs no separate adjustment for this offset.
 */
const TITLE_BAR_CROP_PX = 26

/**
 * Impress scrollbar crop (task 1567 fix pass round 2, 2026-09-27, item 2's
 * OWN follow-up — the lead's review of pass2-verify-impress-dark.png found
 * the native vertical scrollbar still visible on the right edge). Same
 * "crop it in our web shell" convention as `TITLE_BAR_CROP_PX` above, for
 * the same underlying reason: `.uno:ScrollBar` is a CONFIRMED engine no-op
 * for Impress specifically (3 independent verification attempts in
 * bb-office-worker.js's own item-2 comment all showed no effect; Writer and
 * Calc both have real, working per-property scrollbar toggles and need no
 * such crop). MEASURED, not guessed: `pass2-verify-impress-dark.png` was
 * scanned pixel-by-pixel — the native scrollbar's own track (a bright
 * ~250,250,250 column against the app's dark canvas background) starts at
 * x=1266 of a 1280px-wide capture, i.e. the rightmost 14px; this overlay
 * uses 20px for a safety margin, matching the title-bar crop's own
 * "measured width plus a few px" convention. Unlike the title-bar crop
 * (which re-anchors the iframe itself, since that chrome sits ABOVE the
 * document and shifting the whole frame reveals more real canvas at the
 * bottom instead of wasting it), this is a simple opaque overlay div: the
 * scrollbar's own track sits in the dark canvas MARGIN to the right of the
 * slide, never over the slide's own rendered pixels (confirmed in the same
 * scan — the transition at every sampled row goes directly from the dark
 * canvas background to the scrollbar's bright track, with no intervening
 * white slide pixels), so painting over just that margin strip in the SAME
 * canvas-dark color hides the scrollbar without touching, resizing, or
 * reflowing the document canvas underneath it. `useImpressFitZoom` (the
 * OTHER half of this fix pass round) keeps the slide clear of this margin
 * by design (its own `FIT_MARGIN_FACTOR`), so this overlay never needs to
 * cover live content even as the container is resized.
 */
const IMPRESS_SCROLLBAR_CROP_PX = 20

/**
 * The presentational half: renders the iframe once a manifest-derived src is
 * known. `useOfficeEngine` (above) is exported separately so OfficeEditor can
 * own the hook's return value directly instead of prop-drilling through a
 * ref-forwarding wrapper.
 */
export function OfficeEngineFrame({
  iframeSrc,
  iframeRef,
  onLoad,
  className,
  cropScrollbar = false,
}: {
  iframeSrc: string | null
  iframeRef: React.RefObject<HTMLIFrameElement | null>
  onLoad: () => void
  className?: string
  /** Impress only — see `IMPRESS_SCROLLBAR_CROP_PX`'s own comment. */
  cropScrollbar?: boolean
}) {
  if (!iframeSrc) return null
  return (
    <div className={className ?? 'relative h-full w-full overflow-hidden'} data-testid="office-engine-frame-crop">
      <iframe
        ref={iframeRef}
        src={iframeSrc}
        onLoad={onLoad}
        title="Office document canvas"
        data-testid="office-engine-frame"
        // Same-origin by construction (the whole point — see this file's
        // header) — allow-same-origin is required for this component to reach
        // `contentWindow.bbOffice`. allow-scripts lets the engine's own JS
        // (Qt/Emscripten bootstrap, bb-office-api.js) run at all. Nothing else
        // is granted: no allow-popups (SystemShellExecute's product path is
        // dispatched to the HOST page via a CustomEvent, per docs/EGRESS.md —
        // this iframe never needs to navigate/pop up itself), no allow-forms,
        // no allow-top-navigation.
        sandbox="allow-scripts allow-same-origin"
        // clipboard-read/write (task 1567, Calc lane): `.uno:Copy`'s system
        // clipboard write executes in THIS frame's own realm, and without an
        // explicit Permissions-Policy grant here a same-origin sandboxed
        // iframe's clipboard calls are denied (found by actually running the
        // Calc e2e against the real engine: Sum/Average/Count never appeared
        // until this was added). No egress/security concern — the clipboard
        // is a local OS resource, not a network path (docs/EGRESS.md).
        allow="cross-origin-isolated; clipboard-read; clipboard-write"
        className="absolute left-0 right-0 border-0 bg-white"
        // CRITIQUE.md findings #1/#2 (task 1567, 2026-09-27) — SECOND root
        // cause, found by actually instrumenting the real running app (not
        // assumed from the earlier bridge-side fix alone): `<iframe>` is a
        // REPLACED element, and `position:absolute; left:0; right:0` with no
        // explicit `width` does NOT stretch a replaced element to fill its
        // containing block the way it does for a plain `<div>` — browsers
        // fall back to the iframe's own intrinsic default width, 300px
        // (confirmed empirically: a real e2e diagnostic measured this
        // iframe's own `getBoundingClientRect()` at exactly 300×N inside a
        // 1064px-wide canvas area). This is the SAME symptom bb-office-api.js's
        // canvas-backing-store fix targets, but at one level up: that fix
        // makes the CANVAS match ITS OWN document's box; this fix makes that
        // whole document's box (the iframe) actually fill this container in
        // the first place. `height` was already explicit for the crop hack
        // above — `width` needed the same treatment, not left to `left`/
        // `right` alone.
        style={{ top: -TITLE_BAR_CROP_PX, height: `calc(100% + ${TITLE_BAR_CROP_PX}px)`, width: '100%' }}
      />
      {cropScrollbar && (
        <div
          aria-hidden="true"
          data-testid="office-engine-scrollbar-crop"
          className="pointer-events-none absolute inset-y-0 right-0 bg-canvas-dark"
          style={{ width: IMPRESS_SCROLLBAR_CROP_PX }}
        />
      )}
    </div>
  )
}
