// Dev only (batch M2), never bundled: the live state route. `?window=main-app&platform=windows&state=DW-0402` (or the
// tray / onboarding window) mounts the REAL window, with its real sections, handlers and routing, and drives it into
// the frame's state from the one fixture set: the recipe's route and scenario go into the URL, its reads answer the
// commands a browser lacks, its seeds give a component its first UI state, and its steps are replayed through real DOM
// events. The result is written on <html>: data-v2-state, data-v2-state-result ("ready" or "failed: why") and, for a
// state the live app cannot draw yet, data-v2-state-gap. Inside Tauri nothing here runs: the real backend answers.
// The reads reach the app through ./host.ts, the stand-in for Rust behind Tauri's `invoke`.
import { installHost } from './host'
import { setSeeds } from '../seed'
import { answerFrom } from './reads'
import { run } from './driver'
import { RECIPES, type Recipe, type WindowKind } from './states'

let started = false

function targetSearch(id: string, recipe: Recipe): URLSearchParams {
  const q = new URLSearchParams(recipe.q ?? '')
  const out = new URLSearchParams({ window: recipe.window ?? 'main-app', platform: 'windows', state: id })
  for (const keep of ['theme']) { const v = new URLSearchParams(window.location.search).get(keep); if (v) out.set(keep, v) }
  q.forEach((v, k) => out.set(k, v))
  return out
}

/** The window has drawn something real (a lazy section has loaded), plus a moment for its first reads to answer. */
async function rendered(): Promise<void> {
  const until = Date.now() + 8000
  while (Date.now() < until && !(document.getElementById('root')?.innerText ?? '').trim()) await new Promise((r) => setTimeout(r, 60))
  await new Promise((r) => setTimeout(r, 400))
}

const same = (a: URLSearchParams, b: URLSearchParams) => a.size === b.size && [...a].every(([k, v]) => b.get(k) === v)

function mark(id: string, result: string, gap?: string) {
  const d = document.documentElement.dataset
  d.v2State = id
  if (gap) d.v2StateGap = gap
  d.v2StateResult = result
}

/**
 * Runs before the window mounts. Resolves when the window may mount; never resolves when the page is moving to the
 * recipe's own URL (another window, or the recipe's route and scenario), so module-level reads see them.
 */
export function prepare(kind: WindowKind): Promise<void> {
  const params = new URLSearchParams(window.location.search)
  const id = params.get('state')
  const recipe = id ? RECIPES[id] : undefined
  if (id && !recipe) {
    installHost(answerFrom())
    mark(id, `failed: no live state recipe for ${id}`)
    return Promise.resolve()
  }
  if (!id || !recipe) { installHost(answerFrom()); return Promise.resolve() }
  const want = targetSearch(id, recipe)
  if ((recipe.window ?? 'main-app') !== kind || !same(want, params)) {
    window.location.replace(`${window.location.pathname}?${want.toString()}`)
    return new Promise(() => {})
  }
  installHost(answerFrom(recipe.reads, recipe.answers))
  // Each live state starts from a clean PC: the recipe's own view settings, nothing left from another state.
  try {
    for (const k of Object.keys(localStorage)) if (k.startsWith('bb.v2.')) localStorage.removeItem(k)
    for (const [k, v] of Object.entries(recipe.storage ?? {})) localStorage.setItem(k, typeof v === 'string' ? v : JSON.stringify(v))
  } catch { /* storage refused: the recipe's view settings are not applied */ }
  setSeeds(recipe.seeds ?? null)
  if (!started) {
    started = true
    // After the window's first render (StrictMode mounts twice in dev; this runs once).
    void rendered().then(() => {
      void run(recipe.steps ?? []).then((failure) => {
        setSeeds(null)
        // Let the last step's answer render before the audit reads the screen.
        setTimeout(() => mark(id, recipe.native ? `native: ${recipe.native}` : failure ? `failed: ${failure}` : 'ready', recipe.gap), 250)
      })
    })
  }
  return Promise.resolve()
}
