/**
 * TypeScript surface for `window.bbOffice`, as exposed by our LibreOffice-WASM
 * fork's bridge script (repos/office/bridge/bb-office-api.js) on the engine
 * iframe's own `window`. Signatures mirror that file's JSDoc exactly (task
 * 1567 phase 4 delivery note) — this module adds no behavior, only types, so
 * it can be shared between `OfficeEngineHost` (the real iframe) and any test
 * double.
 */

export interface UnoArg {
  name: string
  value: string | number | boolean
}

export interface UnoCommandState {
  isEnabled: boolean
  state: unknown
}

export interface OutlineHeading {
  level: number
  text: string
}

/** CRITIQUE.md finding #8 (task 1567): live word count + cursor-locale
 *  language for the status bar. Writer only — all-null for a non-Writer
 *  active document, matching `getOutline()`'s own convention. */
export interface DocStats {
  words: number | null
  characters: number | null
  language: string | null
}

export type OfficeDocKind = 'writer' | 'calc' | 'impress'

export interface OfficeBridge {
  open(bytes: Uint8Array, filename: string): Promise<{ ext: string; saveExt: string }>
  save(): Promise<Uint8Array>
  insertImage(bytes: Uint8Array, mimeType: string): Promise<{ inserted: true }>
  insertHyperlink(text: string, url: string): Promise<{ inserted: true }>
  dispatch(command: string, args?: UnoArg[]): Promise<{ dispatched: true }>
  onState(command: string, cb: (state: UnoCommandState) => void): Promise<() => void>
  onModifiedChange(cb: (modified: boolean) => void): Promise<() => void>
  onSelectionChange(cb: (sel: { text: string }) => void): Promise<() => void>
  getOutline(): Promise<OutlineHeading[]>
  getDocStats(): Promise<DocStats>
  goToHeading(index: number): Promise<OutlineHeading>
  setZoom(percent: number): Promise<{ zoom: number }>
  /** Fix pass round 2 (task 1567, 2026-09-27, Impress zoom-to-fit): the
   *  active document's page size (1/100 mm), for `use-impress-fit-zoom.ts`'s
   *  own fit-to-container computation. Null for a non-Draw/Impress active
   *  document. */
  getSlideSize(): Promise<{ width: number; height: number } | null>
  newDocument(docKind: OfficeDocKind, templateBytes?: Uint8Array): Promise<{ docKind: OfficeDocKind }>
  /** KNOWN ENGINE GAP (task 1567 phase 4): the config write is real and
   *  persists, but never repaints the CURRENTLY open document or window —
   *  always resolves `appliedLive: false`. OfficeEngineHost seeds theme at
   *  boot, before `open()`/`newDocument()`, to work around this — see its
   *  own comment. Never present this call's return value as "switched" to
   *  a user. */
  setTheme(theme: 'light' | 'dark' | 'auto'): Promise<{ theme: string; appliedLive: false }>
  /** Fix pass item 1 (task 1567): the color LO paints AROUND the document —
   *  distinct from setTheme() above and, unlike it, applies live in this
   *  engine build. rgb is 0xRRGGBB. */
  setWorkspaceColor(rgb: number): Promise<{ applied: boolean; rgb?: number; reason?: string }>
}

/** The iframe's own global, once the bridge script has finished loading. */
export interface OfficeEngineWindow extends Window {
  bbOffice?: OfficeBridge
}
