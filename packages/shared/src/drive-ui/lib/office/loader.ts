/**
 * Office bundle loader (task 1567 — web delivery groundwork).
 *
 * Fetches the LibreOffice-WASM office bundle (`soffice.wasm` / `soffice.data` /
 * `soffice.js` + a handful of small support files) from OUR OWN origin only,
 * at a content-hashed path (`/office/<version>/<file>`). Zero egress is a hard
 * product constraint (docs/EGRESS.md, task 1567) — this module never fetches
 * from any host other than the page's own origin, and asserts that at every
 * fetch call site (see `assertSameOrigin`).
 *
 * This is groundwork: nothing calls `loadOfficeBundle()` yet (the editor
 * itself is a later phase). It exists so the delivery plumbing — manifest
 * fetch, streaming download with real progress, cancellation, and a clear
 * error taxonomy — is built, tested, and ready before the actual bundle is
 * wired into a route.
 *
 * Design notes:
 * - The big binaries (`soffice.wasm` ~164 MB, `soffice.data` ~92 MB) are
 *   downloaded via a plain `fetch()` whose response body is `tee()`'d: one
 *   branch is read in a loop to report byte-level progress, the other is
 *   handed back as a fresh, untouched `Response` — still wired for
 *   `WebAssembly.instantiateStreaming` / `compileStreaming` — before this
 *   module buffers it with `.arrayBuffer()` for the caller. Progress is
 *   derived only from bytes that have already streamed past, never by
 *   buffering the whole file up front.
 * - Integrity is delegated to the platform via `fetch(url, { integrity })`
 *   (the same SRI approach `gen-wasm-sri.mjs` / `crypto.worker.ts` already use
 *   for the core WASM binary) — the browser rejects the response before it
 *   resolves if the hash doesn't match, so this module never has to hash a
 *   164 MB buffer in JS.
 * - Cancellation is a real `AbortController` wired through every fetch,
 *   exposed both as an accepted `signal` option and as a returned `cancel()`.
 */

// ─── Manifest shape ──────────────────────────────────────────────────────────

/** One file in the office bundle, as published under `/office/<version>/`. */
export interface OfficeManifestAsset {
  /** File name relative to the version directory, e.g. "soffice.wasm". */
  path: string
  /** Size in bytes, used for progress totals and sanity checks. */
  bytes: number
  /** `sha256-<base64>` or `sha384-<base64>` — passed straight to `fetch()`'s `integrity`. */
  integrity: string
  /** MIME type the server is expected to serve this file as. */
  contentType: string
}

export interface OfficeManifest {
  /** Content-hash version directory name (also the URL path segment). */
  version: string
  /** ISO timestamp the manifest was generated. */
  generated: string
  assets: OfficeManifestAsset[]
}

function isOfficeManifestAsset(value: unknown): value is OfficeManifestAsset {
  if (!value || typeof value !== 'object') return false
  const v = value as Record<string, unknown>
  return (
    typeof v.path === 'string' && v.path.length > 0 &&
    typeof v.bytes === 'number' && v.bytes >= 0 &&
    typeof v.integrity === 'string' && v.integrity.length > 0 &&
    typeof v.contentType === 'string' && v.contentType.length > 0
  )
}

/** Validate an unknown JSON value as an `OfficeManifest`. Throws `OfficeLoaderError('manifest-invalid')` otherwise. */
export function parseOfficeManifest(value: unknown): OfficeManifest {
  if (!value || typeof value !== 'object') {
    throw new OfficeLoaderError('manifest-invalid', 'Office manifest is not an object')
  }
  const v = value as Record<string, unknown>
  if (typeof v.version !== 'string' || v.version.length === 0) {
    throw new OfficeLoaderError('manifest-invalid', 'Office manifest missing "version"')
  }
  if (!Array.isArray(v.assets) || v.assets.length === 0) {
    throw new OfficeLoaderError('manifest-invalid', 'Office manifest missing non-empty "assets"')
  }
  if (!v.assets.every(isOfficeManifestAsset)) {
    throw new OfficeLoaderError('manifest-invalid', 'Office manifest contains a malformed asset entry')
  }
  return {
    version: v.version,
    generated: typeof v.generated === 'string' ? v.generated : '',
    assets: v.assets as OfficeManifestAsset[],
  }
}

// ─── Errors ──────────────────────────────────────────────────────────────────

export type OfficeLoaderErrorCode =
  | 'manifest-fetch-failed'
  | 'manifest-invalid'
  // Covers BOTH a network-level failure AND a Subresource Integrity mismatch.
  // Browsers reject `fetch(url, { integrity })` with the same generic
  // rejection for both cases (no distinguishable error type/message across
  // engines), so this module does not claim a separate
  // "asset-integrity-mismatch" code it could never actually reach —
  // that would be a code path with no red-proof.
  | 'asset-fetch-failed'
  | 'cross-origin-rejected'
  | 'cancelled'
  | 'unsupported-environment'

/** A clear, typed error state — never a bare rejected promise with no code. */
export class OfficeLoaderError extends Error {
  readonly code: OfficeLoaderErrorCode
  readonly cause?: unknown
  constructor(code: OfficeLoaderErrorCode, message: string, cause?: unknown) {
    super(message)
    this.name = 'OfficeLoaderError'
    this.code = code
    this.cause = cause
  }
}

function isAbortError(err: unknown): boolean {
  return err instanceof DOMException && err.name === 'AbortError'
}

// ─── Same-origin guard ───────────────────────────────────────────────────────

/**
 * Refuses to build a request against any host but the page's own origin.
 * Zero egress is a product hard-constraint (docs/EGRESS.md) — this is the
 * one place every office asset URL is constructed, so it is the one place
 * that guarantee is enforced in code, not just by convention.
 */
export function assertSameOrigin(url: string, currentOrigin: string): string {
  const resolved = new URL(url, currentOrigin)
  if (resolved.origin !== currentOrigin) {
    throw new OfficeLoaderError(
      'cross-origin-rejected',
      `Refused to load office asset from a different origin: ${resolved.origin} (expected ${currentOrigin})`,
    )
  }
  return resolved.toString()
}

// ─── Progress ────────────────────────────────────────────────────────────────

export interface OfficeLoadProgress {
  /** Path of the asset currently transferring, e.g. "soffice.wasm". */
  asset: string
  /** Bytes received for THIS asset so far. */
  assetLoadedBytes: number
  /** Total bytes for this asset (from the manifest). */
  assetTotalBytes: number
  /** Bytes received across the whole bundle so far (completed assets + in-flight). */
  overallLoadedBytes: number
  /** Total bytes across the whole bundle (from the manifest). */
  overallTotalBytes: number
}

// ─── Options ─────────────────────────────────────────────────────────────────

export interface LoadOfficeBundleOptions {
  /** Base path the manifest and assets are served under. Default: "/office". */
  baseUrl?: string
  /** Fired as bytes stream in. Never throws into the caller — errors inside are swallowed. */
  onProgress?: (progress: OfficeLoadProgress) => void
  /** Cancels the whole load. Already-downloaded assets are discarded. */
  signal?: AbortSignal
  /** Injectable for tests; defaults to the global `fetch`. */
  fetchImpl?: typeof fetch
  /** Injectable for tests; defaults to `location.origin` (throws outside a browser unless provided). */
  origin?: string
}

/** One fetched asset: raw bytes plus a fresh `Response` still wired for streaming compile. */
export interface OfficeAssetHandle {
  path: string
  contentType: string
  bytes: ArrayBuffer
}

export interface LoadedOfficeBundle {
  version: string
  manifest: OfficeManifest
  /** Every asset's bytes, keyed by manifest `path`. */
  assets: Map<string, OfficeAssetHandle>
}

// ─── Manifest fetch ──────────────────────────────────────────────────────────

export async function fetchOfficeManifest(
  opts: Pick<LoadOfficeBundleOptions, 'baseUrl' | 'signal' | 'fetchImpl' | 'origin'> = {},
): Promise<OfficeManifest> {
  const baseUrl = opts.baseUrl ?? '/office'
  const origin = opts.origin ?? (typeof location !== 'undefined' ? location.origin : undefined)
  if (!origin) {
    throw new OfficeLoaderError('unsupported-environment', 'No origin available to resolve the office manifest URL')
  }
  const doFetch = opts.fetchImpl ?? fetch
  const url = assertSameOrigin(`${baseUrl}/manifest.json`, origin)

  let res: Response
  try {
    res = await doFetch(url, { signal: opts.signal, credentials: 'omit', cache: 'no-store' })
  } catch (err) {
    if (isAbortError(err)) throw new OfficeLoaderError('cancelled', 'Office manifest fetch was cancelled', err)
    throw new OfficeLoaderError('manifest-fetch-failed', `Failed to fetch office manifest: ${String(err)}`, err)
  }
  if (!res.ok) {
    throw new OfficeLoaderError('manifest-fetch-failed', `Office manifest fetch returned ${res.status}`)
  }

  let json: unknown
  try {
    json = await res.json()
  } catch (err) {
    throw new OfficeLoaderError('manifest-invalid', 'Office manifest was not valid JSON', err)
  }
  return parseOfficeManifest(json)
}

// ─── Single-asset streaming fetch with progress ─────────────────────────────

/**
 * Fetch one asset, tee-ing the response body so `onBytes` gets a live byte
 * count while the returned `Response` is still untouched and safe to hand to
 * `WebAssembly.instantiateStreaming` / `compileStreaming` — or to `.arrayBuffer()`
 * for the non-wasm files.
 */
async function fetchAssetStreaming(
  url: string,
  asset: OfficeManifestAsset,
  onBytes: (loaded: number) => void,
  fetchImpl: typeof fetch,
  signal?: AbortSignal,
): Promise<Response> {
  let res: Response
  try {
    res = await fetchImpl(url, {
      signal,
      credentials: 'omit',
      integrity: asset.integrity,
      cache: 'force-cache',
    })
  } catch (err) {
    if (isAbortError(err)) throw new OfficeLoaderError('cancelled', `Office asset "${asset.path}" fetch was cancelled`, err)
    // A failed SRI check surfaces here too (fetch() rejects rather than resolving with !ok).
    throw new OfficeLoaderError('asset-fetch-failed', `Failed to fetch office asset "${asset.path}": ${String(err)}`, err)
  }
  if (!res.ok) {
    throw new OfficeLoaderError('asset-fetch-failed', `Office asset "${asset.path}" fetch returned ${res.status}`)
  }
  if (!res.body) {
    // No streaming body support (very old runtime) — fall back to a single onBytes call.
    onBytes(asset.bytes)
    return res
  }

  const [progressBranch, passthroughBranch] = res.body.tee()

  // Progress-only reader — errors here never surface to the caller; the
  // passthrough branch is what actually determines success/failure downstream.
  void (async () => {
    const reader = progressBranch.getReader()
    let loaded = 0
    try {
      for (;;) {
        const { done, value } = await reader.read()
        if (done) break
        loaded += value.byteLength
        onBytes(loaded)
      }
    } catch {
      // Best-effort progress only.
    }
  })()

  return new Response(passthroughBranch, { headers: res.headers, status: res.status, statusText: res.statusText })
}

// ─── Whole-bundle load ───────────────────────────────────────────────────────

/**
 * Load the manifest, then every listed asset, reporting byte-level progress
 * across the whole bundle. Fully cancellable via `opts.signal` — cancelling
 * mid-download rejects with `OfficeLoaderError('cancelled', …)` and no partial
 * asset is returned.
 */
export async function loadOfficeBundle(opts: LoadOfficeBundleOptions = {}): Promise<LoadedOfficeBundle> {
  const baseUrl = opts.baseUrl ?? '/office'
  const origin = opts.origin ?? (typeof location !== 'undefined' ? location.origin : undefined)
  if (!origin) {
    throw new OfficeLoaderError('unsupported-environment', 'No origin available to resolve office asset URLs')
  }
  const fetchImpl = opts.fetchImpl ?? fetch

  const manifest = await fetchOfficeManifest({ baseUrl, signal: opts.signal, fetchImpl, origin })

  const overallTotalBytes = manifest.assets.reduce((sum, a) => sum + a.bytes, 0)
  const perAssetLoaded = new Map<string, number>(manifest.assets.map((a) => [a.path, 0]))

  function reportProgress(asset: OfficeManifestAsset, loaded: number) {
    perAssetLoaded.set(asset.path, loaded)
    if (!opts.onProgress) return
    let overallLoadedBytes = 0
    for (const v of perAssetLoaded.values()) overallLoadedBytes += v
    try {
      opts.onProgress({
        asset: asset.path,
        assetLoadedBytes: loaded,
        assetTotalBytes: asset.bytes,
        overallLoadedBytes,
        overallTotalBytes,
      })
    } catch {
      // A throwing progress callback must never abort the load.
    }
  }

  const assets = new Map<string, OfficeAssetHandle>()
  for (const asset of manifest.assets) {
    if (opts.signal?.aborted) {
      throw new OfficeLoaderError('cancelled', 'Office bundle load was cancelled')
    }
    const url = assertSameOrigin(`${baseUrl}/${manifest.version}/${asset.path}`, origin)
    const res = await fetchAssetStreaming(
      url,
      asset,
      (loaded) => reportProgress(asset, loaded),
      fetchImpl,
      opts.signal,
    )
    const bytes = await res.arrayBuffer()
    assets.set(asset.path, { path: asset.path, contentType: asset.contentType, bytes })
  }

  return { version: manifest.version, manifest, assets }
}

// ─── Cancellable wrapper ─────────────────────────────────────────────────────

export interface CancellableOfficeLoad {
  promise: Promise<LoadedOfficeBundle>
  cancel: () => void
}

/**
 * Convenience wrapper for call sites that want an explicit `cancel()` handle
 * (e.g. a "Cancel" button in the loading skeleton) instead of managing their
 * own `AbortController`.
 */
export function createCancellableOfficeLoad(
  opts: Omit<LoadOfficeBundleOptions, 'signal'> = {},
): CancellableOfficeLoad {
  const controller = new AbortController()
  const promise = loadOfficeBundle({ ...opts, signal: controller.signal })
  return { promise, cancel: () => controller.abort() }
}
