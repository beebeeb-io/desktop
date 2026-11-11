// The banners under the toolbar (DW-0101 to DW-0106). One at a time, in this order: signed out, storage full,
// sync problem, backup problem, storage almost full, offline. Neutral surfaces; no amber (amber is for
// encryption state and the primary action only).

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { useDriveSource, type StorageSummary, type SyncSummary } from '@beebeeb/shared/drive-ui'
import { useDriveExtras, type DriveStatus } from '../source/extras'
import { navigateTo, type Route, type SectionId } from '../routes'

export type BannerKind = 'signed-out' | 'storage-full' | 'sync-problem' | 'backup-problem' | 'storage-almost-full' | 'offline'

/** The board's order: the first active kind wins. */
export const BANNER_ORDER: BannerKind[] = ['signed-out', 'storage-full', 'sync-problem', 'backup-problem', 'storage-almost-full', 'offline']

export interface BannerInput {
  signedOut?: boolean
  storage?: StorageSummary | null
  syncProblems?: number
  backupProblems?: number
  offline?: boolean
  /** Kinds the person dismissed (each comes back when it happens again). */
  dismissed?: ReadonlySet<BannerKind>
}

/** Every kind whose condition holds, in the board's order. */
export function activeKinds(input: BannerInput): BannerKind[] {
  const used = input.storage ? input.storage.usedBytes / Math.max(1, input.storage.quotaBytes) : 0
  const active: Record<BannerKind, boolean> = {
    'signed-out': !!input.signedOut,
    'storage-full': used >= 1,
    'sync-problem': (input.syncProblems ?? 0) > 0,
    'backup-problem': (input.backupProblems ?? 0) > 0,
    'storage-almost-full': used >= 0.9 && used < 1,
    offline: !!input.offline,
  }
  return BANNER_ORDER.filter((k) => active[k])
}

export function activeBanner(input: BannerInput): BannerKind | null {
  return activeKinds(input).find((k) => !input.dismissed?.has(k)) ?? null
}

const DISMISSABLE: ReadonlySet<BannerKind> = new Set(['sync-problem', 'backup-problem', 'storage-almost-full'])

function BannerIcon({ kind }: { kind: BannerKind }) {
  const props = { width: 18, height: 18, viewBox: '0 0 18 18', fill: 'none', stroke: 'currentColor', strokeWidth: 1.4, strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true } as const
  if (kind === 'offline') return <svg {...props}><path d="M2 7a10 10 0 0 1 14 0M4.5 9.8a6.5 6.5 0 0 1 9 0M7 12.6a3 3 0 0 1 4 0" /><path d="M2.5 2.5l13 13" /></svg>
  if (kind === 'signed-out') return <svg {...props}><rect x="3.5" y="8" width="11" height="7.5" rx="1.5" /><path d="M6 8V6a3 3 0 0 1 6 0v2" /></svg>
  if (kind === 'storage-full' || kind === 'storage-almost-full') return <svg {...props}><ellipse cx="9" cy="4.5" rx="6" ry="2.5" /><path d="M3 4.5v9c0 1.4 2.7 2.5 6 2.5s6-1.1 6-2.5v-9M3 9c0 1.4 2.7 2.5 6 2.5S15 10.4 15 9" /></svg>
  return <svg {...props}><circle cx="9" cy="9" r="6.5" /><path d="M9 5.5v4M9 12v.1" /></svg>
}

interface BannerCopy { text: string; detail: string; action?: string }

export function bannerCopy(kind: BannerKind, o: { syncProblems?: number; backupProblems?: number; storage?: StorageSummary | null; asOf?: string } = {}): BannerCopy {
  const n = (v: number | undefined) => v ?? 3
  // Decimal units, as the meter and the web app: 1 GB = 1000³ bytes (a binary value is never labelled GB).
  const used = o.storage ? Math.floor(o.storage.usedBytes / 1000 ** 3) : 950
  const quota = o.storage ? Math.round(o.storage.quotaBytes / 1000 ** 3) / 1000 : 1
  switch (kind) {
    // DW-1108 Shows the sentence in full: "Changes you make will upload when you’re back." Typographic apostrophes, as
    // the rest of the window's copy (DW-0109's "Couldn’t", DW-0410b's "There’s").
    case 'offline': return { text: 'You’re offline.', detail: `This list is from ${o.asOf ?? 'today at 09:12'}. Changes you make will upload when you’re back.` }
    case 'sync-problem': return { text: `${n(o.syncProblems)} ${n(o.syncProblems) === 1 ? 'item' : 'items'} can’t sync.`, detail: 'Everything else is up to date.', action: 'View' }
    case 'storage-almost-full': return { text: 'Your storage is almost full.', detail: `${used} GB of ${quota} TB used.`, action: 'Storage' }
    case 'storage-full': return { text: 'Your storage is full.', detail: 'New uploads and backups are paused. Your files are safe.', action: 'Storage' }
    case 'signed-out': return { text: 'Backup and sync have stopped.', detail: 'Enter your password to continue. Nothing has been lost.', action: 'Sign in' }
    case 'backup-problem': return { text: `${n(o.backupProblems)} ${n(o.backupProblems) === 1 ? 'item' : 'items'} couldn’t back up.`, detail: 'Everything else is backed up.', action: 'View' }
  }
}

export function Banner({ kind, copy, onAction, onDismiss }: { kind: BannerKind; copy: BannerCopy; onAction?: () => void; onDismiss?: () => void }) {
  return (
    <div role="status" data-banner={kind} className="flex min-h-10 flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 py-2 text-[13px] leading-5 text-ink-2">
      <span className="flex-none text-ink-3"><BannerIcon kind={kind} /></span>
      <span className="min-w-0 flex-1">
        <span className="font-medium text-ink">{copy.text}</span> <span>{copy.detail}</span>
      </span>
      {copy.action && (
        <button type="button" onClick={onAction} className="h-7 flex-none rounded-md border border-line bg-paper px-3 text-[13px] font-medium text-ink hover:bg-paper-3 focus-visible:outline-2 focus-visible:outline-ink-3">
          {copy.action}
        </button>
      )}
      {onDismiss && DISMISSABLE.has(kind) && (
        <button type="button" aria-label="Dismiss" title="Dismiss" onClick={onDismiss} className="grid h-7 w-7 flex-none place-items-center rounded-md text-ink-3 hover:bg-paper-3 focus-visible:outline-2 focus-visible:outline-ink-3">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2" aria-hidden="true"><path d="M.5.5l9 9M9.5.5l-9 9" /></svg>
        </button>
      )}
    </div>
  )
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
/** How old the list on this PC is, in the banner's words: "today at 09:12", "yesterday at 22:40", "on 3 Nov at 08:00". */
export function listAge(iso: string, now: Date): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return 'before the connection dropped'
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  // Calendar days (UTC of the local date): a DST change between the two midnights does not shift the count.
  const day = (x: Date) => Date.UTC(x.getFullYear(), x.getMonth(), x.getDate()) / 86_400_000
  const ago = day(now) - day(d)
  if (ago === 0) return `today at ${hm}`
  if (ago === 1) return `yesterday at ${hm}`
  return `on ${d.getDate()} ${MONTHS[d.getMonth()]} at ${hm}`
}

/** Where each banner's button goes: the frame it names. */
export const BANNER_TARGET: Partial<Record<BannerKind, string>> = {
  'sync-problem': 'DW-0428',
  'storage-almost-full': 'DW-0712',
  'storage-full': 'DW-0712',
  'signed-out': 'DW-1109',
  'backup-problem': 'DW-0741',
}
/**
 * The route of that frame: Sync history on Problems (DW-0428), Settings › Storage (DW-0712), Settings › Backups'
 * Couldn't back up (DW-0741). Sign in has none: the password sign-in over the list kept on this PC (DW-1109 → DW-A03)
 * is B10's, and the window has no entry to it yet, so its button stays an event (M3a: listed as blocked).
 */
export const BANNER_ROUTE: Partial<Record<BannerKind, Partial<Route> & { section: SectionId }>> = {
  'sync-problem': { section: 'sync-history', sub: 'problems' },
  'storage-almost-full': { section: 'settings', sub: 'storage' },
  'storage-full': { section: 'settings', sub: 'storage' },
  'backup-problem': { section: 'settings', sub: 'backup-failures' },
}
export const BANNER_ACTION_EVENT = 'bb:banner-action'

/** A banner's button: its route when it has one, else the event. */
export function followBanner(kind: BannerKind): void {
  const route = BANNER_ROUTE[kind]
  if (route) navigateTo(route)
  else window.dispatchEvent(new CustomEvent(BANNER_ACTION_EVENT, { detail: { kind, frame: BANNER_TARGET[kind] } }))
}

// The banner sits under the toolbar on every view (DW-0101 to DW-0106 Rules). The window mounts one GlobalBanner; a
// section with a toolbar puts a BannerSlot under it and the banner is drawn there. A view without a slot gets it under
// the caption bar, so it is never lost.
// Every mounted slot, the newest last: the banner goes to the newest, and back to the one before when it unmounts.
let slots: HTMLElement[] = []
const slotListeners = new Set<() => void>()
const subscribeSlot = (l: () => void) => { slotListeners.add(l); return () => { slotListeners.delete(l) } }
const currentSlot = () => slots[slots.length - 1] ?? null

/** Where the window's banner is drawn: right under a section's toolbar. */
export function BannerSlot() {
  const mine = useRef<HTMLElement | null>(null)
  const ref = useCallback((el: HTMLDivElement | null) => {
    if (el) { mine.current = el; slots = [...slots.filter((s) => s !== el), el] }
    else if (mine.current) { const gone = mine.current; slots = slots.filter((s) => s !== gone); mine.current = null }
    slotListeners.forEach((l) => l())
  }, [])
  return <div ref={ref} data-banner-slot className="contents" />
}

/** The live banner of the window: storage and sync from the DriveSource; the signed-out state and the sync and
 *  backup problem counts from B02b's `drive_status` (refreshed with each sync event). `extra` overrides them. */
export function GlobalBanner({ extra }: { extra?: Pick<BannerInput, 'signedOut' | 'syncProblems' | 'backupProblems'> }): ReactNode {
  const source = useDriveSource()
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [sync, setSync] = useState<SyncSummary | null>(null)
  const extras = useDriveExtras()
  const [status, setStatus] = useState<DriveStatus | null>(null)
  const [dismissed, setDismissed] = useState<ReadonlySet<BannerKind>>(new Set())
  useEffect(() => {
    const readStatus = () => extras.status().then(setStatus, () => setStatus(null))
    source.storage().then(setStorage, () => setStorage(null))
    source.sync().then(setSync, () => setSync(null))
    void readStatus()
    return source.subscribe((e) => {
      if (e.kind === 'storage') setStorage(e.storage)
      if (e.kind === 'sync') { setSync(e.sync); void readStatus() }
    })
  }, [source, extras])
  const input: BannerInput = useMemo(() => ({
    signedOut: status?.signedOut, syncProblems: status?.syncProblems, backupProblems: status?.backupProblems,
    ...extra, storage, offline: sync?.phase === 'offline',
  }), [extra, status, storage, sync])
  const kind = activeBanner({ ...input, dismissed })
  const target = useSyncExternalStore(subscribeSlot, currentSlot, () => null)
  // The offline banner names when the list was last fresh: the list's own time (the copy on this PC), else the last
  // sync summary heard while online; never a guess.
  const [lastOnline, setLastOnline] = useState<string | null>(null)
  const [listAsOf, setListAsOf] = useState<string | null>(null)
  useEffect(() => {
    if (sync && sync.phase !== 'offline') setLastOnline(new Date().toTimeString().slice(0, 5))
  }, [sync])
  const offline = sync?.phase === 'offline'
  useEffect(() => {
    if (!offline) { setListAsOf(null); return }
    let live = true
    source.list(null).then((l) => { if (live) setListAsOf(l.asOf ?? null) }, () => { if (live) setListAsOf(null) })
    return () => { live = false }
  }, [source, offline])
  // A dismissed banner comes back when it happens again: forget a dismissal once its condition clears.
  useEffect(() => {
    const live = new Set(activeKinds(input))
    setDismissed((d) => (d.size === 0 || [...d].every((k) => live.has(k)) ? d : new Set([...d].filter((k) => live.has(k)))))
  }, [input])
  if (!kind) return null
  const banner = (
    <Banner
      kind={kind}
      copy={bannerCopy(kind, { syncProblems: input.syncProblems, backupProblems: input.backupProblems, storage, asOf: listAsOf ? listAge(listAsOf, new Date()) : lastOnline ? `today at ${lastOnline}` : 'before the connection dropped' })}
      onAction={() => followBanner(kind)}
      onDismiss={() => setDismissed((d) => new Set(d).add(kind))}
    />
  )
  return target ? createPortal(banner, target) : banner
}
