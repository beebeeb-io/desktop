// Open: one file in the Drive window's content area, under the window's toolbar (DW-1501..1527). Office is B08's.
// Reads and saves only through useDriveSource(). The route is `section=open&id=<file>[&sub=edit]`; `sub=new-file&id=<folder>`.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { DriveSourceError, MAX_EDITABLE_BYTES, useDriveSource, type DriveItem, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { useCommand } from '../../shortcuts'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { setStatusNote } from '../../chrome/statusNote'
import { failedWords } from '../photos/Dialogs'
import { ShareHost, VersionsHost } from './hosts'
import { DetailsRail, MoreButton, NewerBanner, OpenToolbar, ToolbarButton } from './Chrome'
import { EditConflictDialog, editMeta, NewFileNameDialog, NewFileTypeDialog, nameError, FILE_TYPES, openEditorFind, SaveButton, SavedMessage, TextEditor, UnsavedAlert, type FileType } from './Editing'
import { DecryptFailedView, DecryptingView, noRoomWords, OpenInAppNote, TooLargeView } from './States'
import { useDriveExtras } from '../../source/extras'
import { isTooLarge } from '../../source/desktop'
import { ArchiveViewer, AudioPlayer, Centered, CodeViewer, CsvViewer, ImageViewer, MarkdownViewer, RawViewer, type Track } from './Viewers'
import { createKeptCopy, fmtBytes, hhmm, keyBelongsElsewhere, languageLabel, lineCount, READ_ONLY_LINES, saveNoteText, tagContent, viewerKind } from './model'
import * as player from './player'
import { isOfficeFile, NewDocumentFlow, OfficeEditor } from './office'
import { findGoesTo, useOwnFind } from './find'
import { useVersionWatch } from './versionWatch'
import { rawThumbCache } from './thumbCache'

type Loaded =
  | { kind: 'text'; text: string; versionId: string }
  | { kind: 'bytes'; bytes: Uint8Array; versionId: string }
  | { kind: 'none' }

/** `loading` carries the item once its details are in, so Decrypting names it and Cancel knows its folder (DW-1516). */
/** `ready` carries Rust's version number of what was read (drive_details' versionCount); `too-large` the free space
 *  when it was read for the decision. */
type Phase = { name: 'loading'; item: DriveItem | null } | { name: 'ready'; item: DriveItem; data: Loaded; version: number } | { name: 'too-large'; item: DriveItem; free?: number } | { name: 'failed'; item: DriveItem | null }

const TEXTUAL = new Set(['code', 'text', 'markdown', 'csv'])

export default function Section({ nav }: SectionProps) {
  // Ctrl+F: a view's own find, else Search; never Search from an editor, which would drop its edit (find.ts).
  useCommand('find', useCallback(() => { if (findGoesTo(nav.route.sub) === 'search') nav.go({ section: 'search' }) }, [nav]))
  const { id, sub } = nav.route
  if (sub === 'new-file') return <NewFileFlow parentId={id} nav={nav} />
  if (sub === 'new-document') return <NewDocumentFlow parentId={id} nav={nav} />
  if (sub === 'office' && id) return <OfficeEditor key={id} target={{ kind: 'file', id }} nav={nav} />
  if (!id) return <Centered><p className="m-0 text-[13px] text-ink-3">Nothing is open.</p></Centered>
  return <OpenFile key={id} id={id} editing={sub === 'edit'} nav={nav} />
}

function useLoad(id: string) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [phase, setPhase] = useState<Phase>({ name: 'loading', item: null })
  const [tries, setTries] = useState(0)
  // A file this window has just made (New file) has a provisional id until the engine indexes it: wait for that.
  const [unindexed, setUnindexed] = useState(false)
  useEffect(() => {
    let live = true
    let known: DriveItem | null = null
    setUnindexed(false)
    setPhase((p) => ({ name: 'loading', item: 'item' in p ? p.item : null }))
    ;(async () => {
      try {
        const { item, versionCount } = await source.details(id)
        known = item
        if (!live) return
        // A file not on this PC is decrypted here to open it: with less free space than it needs, say so (DW-1517).
        // Only then is the free space read (drive_status counts the state DB; a file on this PC never needs it).
        if (item.local === 'online-only') {
          const free = await extras.status().then((s) => s.freeSpaceBytes, () => undefined)
          if (!live) return
          if (needsRoom(item, free)) return setPhase({ name: 'too-large', item, free })
        }
        setPhase({ name: 'loading', item })
        const kind = viewerKind(item.name)
        // A RAW shows the camera's preview (its thumbnail): the file itself is not read.
        const data: Loaded = TEXTUAL.has(kind) ? { kind: 'text', ...(await source.readText(id)) } : kind === 'other' || kind === 'raw' ? { kind: 'none' } : { kind: 'bytes', ...(await source.readBytes(id)) }
        if (live) setPhase({ name: 'ready', item, data, version: versionCount })
      } catch (e) {
        if (!live) return
        // No room to decrypt it (quota: the disk is full), or over the read cap: the too-large state (DW-1517).
        if ((e instanceof DriveSourceError && e.code === 'quota') || isTooLarge(e)) {
          const { item } = await source.details(id).catch(() => ({ item: null }))
          if (!live) return
          if (item) return setPhase({ name: 'too-large', item })
        }
        if (isProvisional(id) && e instanceof DriveSourceError && e.code === 'not-found') return setUnindexed(true)
        setPhase({ name: 'failed', item: known })
      }
    })()
    return () => { live = false }
  }, [source, extras, id, tries])
  // The engine's index event, never a timer: read again when the engine says something changed (#273 finding 11).
  useEffect(() => {
    if (!unindexed) return
    return source.subscribe((e) => { if (e.kind === 'changed' || e.kind === 'item') setTries((n) => n + 1) })
  }, [source, unindexed])
  return { phase, retry: () => setTries((n) => n + 1), setPhase }
}

/** Rust's stand-in id for a file this PC has just written and the engine has not indexed yet (drive_v2 item_at_path). */
export const isProvisional = (id: string) => id.startsWith('local:')

/** Opening `item` decrypts it on this PC and `free` bytes (when known) are not enough for it. */
export const needsRoom = (item: Pick<DriveItem, 'local' | 'size'>, free: number | undefined) =>
  item.local === 'online-only' && free !== undefined && (item.size ?? 0) > free

function OpenFile({ id, editing, nav }: { id: string; editing: boolean; nav: SectionProps['nav'] }) {
  const source = useDriveSource()
  const readTextTag = useCallback((x: string) => source.readText(x).then((d) => d.versionId), [source])
  const { phase, retry, setPhase } = useLoad(id)
  const watch = useVersionWatch(source, id, phase.name === 'ready' ? phase.version : null, {
    readTag: readTextTag,
    // The restore landed: read the file again and say it is the restore (#273 finding 8).
    restored: (version) => {
      showToast({ kind: 'done', text: `Version restored. This is now version ${version}.` })
      setStatusNote(`Version restored · version ${version} · ${hhmm(new Date().toISOString())}`)
      retry()
    },
    // The save is on the server: what is shown carries Rust's new tag, so a later edit starts from its number.
    saved: (version, tag) => setPhase((p) => (p.name === 'ready' && p.data.kind === 'text' && tagContent(p.data.versionId) === tagContent(tag) ? { ...p, data: { ...p.data, versionId: tag }, version } : p)),
  })
  const { newer, note, dismiss: dismissNewer, savedHere, clearNote } = watch
  const [versionsOpen, setVersionsOpen] = useState(false)
  const [panel, setPanel] = useState(false)
  // DW-1524: after a save the note shows rendered, with the save's state; the window's status line says it too, until
  // the file is left or edited again.
  const savedAt = useRef('')
  useEffect(() => () => setStatusNote(null), [])
  useEffect(() => {
    if (editing || note === null) setStatusNote(null)
    else setStatusNote(`${saveNoteText(note)} · ${savedAt.current}`)
  }, [editing, note])
  useCommand('toggle-details-panel', useCallback(() => setPanel((p) => !p), []))
  // "Back to the folder" (the toolbar's back): the folder the file is in, whatever led here (DW-1503, DW-1507).
  const toFolder = (item: DriveItem | null) => (item ? nav.go({ section: 'files', id: item.parentId }) : nav.back())

  if (phase.name === 'loading') {
    // Cancel stops the read; nothing half-decrypted is kept (Rust writes nothing until the read is whole).
    return <Frame name={phase.item?.name ?? ''} onBack={() => toFolder(phase.item)}><DecryptingView name={phase.item?.name ?? ''} onCancel={() => toFolder(phase.item)} /></Frame>
  }
  if (phase.name === 'failed') return <Frame name={phase.item?.name ?? ''} onBack={() => toFolder(phase.item)}><DecryptFailedView onRetry={retry} onReport={() => nav.go({ section: 'settings', sub: 'report' })} /></Frame>
  if (phase.name === 'too-large') {
    const item = phase.item
    return (
      <Frame name={item.name} onBack={() => toFolder(item)}>
        <TooLargeLive name={item.name} size={item.size ?? 0} known={phase.free} onFreeUp={() => nav.go({ section: 'settings', sub: 'free-up' })}
          onOpenElsewhere={() => void source.open(item.id).catch(() => showToast({ kind: 'failed', text: `Couldn’t open ${item.name} in another app.` }))}
          onDownloadTo={() => showToast({ kind: 'failed', text: 'Saving a decrypted copy elsewhere isn’t available in this version of Beebeeb.' })} />
      </Frame>
    )
  }
  // Office files open in the editor by default (DW-1562); CSV keeps its table viewer.
  if (isOfficeFile(phase.item) && viewerKind(phase.item.name) !== 'csv') return <OfficeEditor target={{ kind: 'file', id }} nav={nav} />
  const { item, data } = phase
  const showNewer = () => { dismissNewer(); clearNote(); retry() }
  const banner = newer && !editing
    ? <NewerBanner text={`A newer version arrived from ${newer.deviceName ?? 'another device'}, ${hhmm(newer.createdAt)}.`} onShow={showNewer} />
    : note !== null && !editing ? <SavedMessage note={note} onVersions={() => setVersionsOpen(true)} /> : null
  const aside = panel ? <DetailsAside id={id} onClose={() => setPanel(false)} /> : null
  const saved = (d: { text: string; versionId: string }) => {
    savedAt.current = hhmm(new Date().toISOString())
    setPhase({ name: 'ready', item, data: { kind: 'text', ...d }, version: phase.version })
    savedHere(d.versionId)
    nav.go({ section: 'open', id: item.id, sub: null })
  }
  const body = editing && data.kind === 'text'
    ? <Editor item={item} initial={data} onSaved={saved} onLeave={(reload) => { nav.go({ section: 'open', id: item.id, sub: null }); if (reload) { dismissNewer(); retry() } }} />
    : <Viewer item={item} data={data} nav={nav} banner={banner} onBack={() => toFolder(item)} onDetails={() => setPanel((p) => !p)} onEdit={clearNote} />
  return (
    <div className="flex min-h-0 flex-1">
      {body}{aside}
      {versionsOpen && <VersionsHost item={item} onClose={() => setVersionsOpen(false)} onRestored={watch.restoredHere} />}
    </div>
  )
}


function DetailsAside({ id, onClose }: { id: string; onClose: () => void }) {
  const source = useDriveSource()
  const [rows, setRows] = useState<[string, string][]>([])
  useEffect(() => {
    void source.details(id).then(({ item, path, versionCount }) => setRows([
      ['Size', item.size === null ? '' : fmtBytes(item.size)], ['Modified', new Date(item.modifiedAt).toLocaleString('en-GB')],
      ['Where', path.map((c) => c.name).join(' / ') || 'Beebeeb'], ['Version history', `${versionCount} version${versionCount === 1 ? '' : 's'}`],
    ]))
  }, [source, id])
  return <DetailsRail rows={rows} onClose={onClose} />
}

function Frame({ name, onBack, children }: { name: string; onBack: () => void; children: ReactNode }) {
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col"><OpenToolbar name={name} onBack={onBack} />{children}</div>
}

function useBlobUrl(bytes: Uint8Array | null, mime: string): string | null {
  const url = useMemo(() => (bytes ? URL.createObjectURL(new Blob([bytes as BlobPart], { type: mime })) : null), [bytes, mime])
  useEffect(() => () => { if (url) URL.revokeObjectURL(url) }, [url])
  return url
}

function Viewer({ item, data, nav, banner, onBack, onDetails, onEdit }: { item: DriveItem; data: Loaded; nav: SectionProps['nav']; banner: ReactNode; onBack: () => void; onDetails: () => void; onEdit: () => void }) {
  const source = useDriveSource()
  const kind = viewerKind(item.name)
  const [wrap, setWrap] = useState(false)
  const [mdMode, setMdMode] = useState<'rendered' | 'source'>('rendered')
  const [share, setShare] = useState(false)
  const bytes = data.kind === 'bytes' ? data.bytes : null
  // A PDF is never framed: the window's CSP refuses a `blob:` frame (lead's M3d ruling 5), so it opens in another app
  // until the page view is drawn in the window (pdf.js, the P1b lane). #273 finding 1.
  const url = useBlobUrl(kind === 'pdf' ? null : bytes, item.mime ?? 'application/octet-stream')
  const text = data.kind === 'text' ? data.text : ''
  const big = (item.size ?? 0) > MAX_EDITABLE_BYTES
  const openElsewhere = () => { source.open(item.id).catch((e: unknown) => showToast({ kind: 'failed', text: failedWords(e, 'open it in another app') })) }
  const edit = () => { onEdit(); nav.go({ section: 'open', id: item.id, sub: big ? null : 'edit' }) }
  // Copy says so: the clipboard gives no sign of its own (DW-1512).
  const copy = () => { navigator.clipboard?.writeText(text).then(() => showToast({ kind: 'done', text: 'Copied' }), () => showToast({ kind: 'failed', text: 'Couldn’t copy. Try again.' })) }
  const openFile = (name: string) => void source.search(name).then((r) => { const hit = r.find((i) => i.name === name); if (hit) nav.go({ section: 'open', id: hit.id }); else showToast({ kind: 'failed', text: `Couldn’t find ${name} in Beebeeb.` }) })
  const textual = TEXTUAL.has(kind)
  const editable = (kind === 'code' || kind === 'text' || kind === 'markdown') && !big
  const pictures = kind === 'raw' || kind === 'image' || kind === 'audio'
  useCommand('edit', useCallback(() => { if (editable) edit(); else if (kind === 'csv') nav.go({ section: 'open', id: item.id, sub: 'office' }) }, [editable, kind])) // eslint-disable-line react-hooks/exhaustive-deps

  const keep = () => source.setAvailableOffline(item.id, true).then(() => showToast({ kind: 'done', text: 'Keeping it on this PC' }), (e: unknown) => showToast({ kind: 'failed', text: failedWords(e, 'keep it on this PC') }))
  const trash = () => source.moveToTrash([item.id]).then(({ undoToken }) => {
    showToast({ kind: 'undo', text: 'Moved to Trash', onAction: () => { source.undoTrash(undoToken).then(() => nav.go({ section: 'open', id: item.id }), () => showToast({ kind: 'failed', text: 'Couldn’t bring it back' })) } })
    onBack()
  }, (e: unknown) => showToast({ kind: 'failed', text: failedWords(e, 'move it to Trash') }))

  const meta = kind === 'code' || kind === 'text' ? `${languageLabel(item.name)} · ${lineCount(text)} lines · UTF-8` : kind === 'raw' || kind === 'audio' ? <FolderMeta item={item} /> : undefined
  const actions = (
    <>
      {kind === 'markdown' && (
        <span role="group" aria-label="View" className="flex rounded-md border border-line p-0.5 text-[12px]">
          {(['rendered', 'source'] as const).map((m) => <button key={m} type="button" aria-pressed={mdMode === m} onClick={() => setMdMode(m)} className={`h-7 rounded px-3 capitalize ${mdMode === m ? 'bg-paper-3 text-ink' : 'text-ink-3'}`}>{m === 'rendered' ? 'Rendered' : 'Source'}</button>)}
        </span>
      )}
      {(kind === 'code' || kind === 'text') && <><ToolbarButton label="Wrap" pressed={wrap} onClick={() => setWrap((w) => !w)} /><ToolbarButton label="Copy" onClick={copy} /></>}
      {kind === 'pdf' && <><ToolbarButton label="Open in another app…" onClick={openElsewhere} /><ToolbarButton label="Share" onClick={() => setShare(true)} /></>}
      {pictures && <><ToolbarButton label="Share" onClick={() => setShare(true)} /><ToolbarButton label="Offline" onClick={() => void keep()} /><ToolbarButton label="Details" onClick={onDetails} /><ToolbarButton label="Trash" onClick={() => void trash()} /></>}
      {kind === 'archive' && <ToolbarButton label="Open in another app…" onClick={openElsewhere} />}
      {(editable || kind === 'csv') && <ToolbarButton label="Edit" shortcut="Ctrl+E" onClick={kind === 'csv' ? () => nav.go({ section: 'open', id: item.id, sub: 'office' }) : edit} />}
      <MoreButton />
    </>
  )

  let body: ReactNode
  if (kind === 'raw') body = <RawView item={item} nav={nav} onOpenElsewhere={openElsewhere} />
  else if (kind === 'image') body = url ? <ImageViewer src={url} name={item.name} /> : null
  else if (kind === 'audio') body = <AudioView item={item} bytes={bytes} nav={nav} />
  else if (kind === 'markdown') body = <MarkdownViewer text={text} mode={mdMode} onOpenFile={openFile} />
  else if (kind === 'csv') body = <CsvViewer text={text} />
  else if (kind === 'archive') body = <ZipView bytes={bytes} />
  else if (textual) body = <>{big && <OpenInAppNote onOpen={openElsewhere}>Read-only: this file is larger than 2 MB.</OpenInAppNote>}<CodeViewer text={text} wrap={wrap} limit={big ? READ_ONLY_LINES : undefined} />{big && <div className="flex-none border-t border-line px-4 py-1.5 font-mono text-[12px] text-ink-3">Showing the first {Math.min(READ_ONLY_LINES, lineCount(text))} of {lineCount(text).toLocaleString('en-GB')} lines</div>}</>
  else body = <Centered><p className="m-0 text-[13px] text-ink-2">{item.name} opens in another app.</p><ToolbarButton label="Open in another app…" onClick={openElsewhere} /></Centered>

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <OpenToolbar name={item.name} meta={meta} onBack={onBack}>{actions}</OpenToolbar>
      {banner}
      {body}
      {share && <ShareHost items={[item]} onClose={() => setShare(false)} />}
    </div>
  )
}

/** "Shoot selects · Sunday 14:20": the folder and when the file changed (DW-1501, DW-1503). */
function FolderMeta({ item }: { item: DriveItem }) {
  const source = useDriveSource()
  const [where, setWhere] = useState<string | null>(null)
  useEffect(() => { void source.details(item.id).then((d) => setWhere(d.path.length > 1 ? d.path[d.path.length - 1].name : 'Files'), () => setWhere(null)) }, [source, item.id])
  return <>{[where, whenLabel(item.modifiedAt)].filter(Boolean).join(' · ')}</>
}

/** The folder's other files of the same viewer, in the folder's order (the RAW's strip, the recording's Up next). */
function useSiblings(item: DriveItem, kinds: string[]): DriveItem[] {
  const source = useDriveSource()
  const [rows, setRows] = useState<DriveItem[]>([item])
  useEffect(() => {
    let live = true
    source.list(item.parentId).then((l) => {
      const same = l.items.filter((i) => !i.isFolder && kinds.includes(viewerKind(i.name)))
      if (live) setRows(same.some((i) => i.id === item.id) ? same : [item, ...same])
    }, () => { if (live) setRows([item]) })
    return () => { live = false }
  }, [source, item]) // eslint-disable-line react-hooks/exhaustive-deps -- kinds is a literal per viewer
  return rows
}

/** How many photos on each side of the open one the strip shows. */
const STRIP_REACH = 8

/** DW-1501: a RAW shows the preview the camera stored in it (the item's thumbnail), and says so; ← → step through the folder. */
function RawView({ item, nav, onOpenElsewhere }: { item: DriveItem; nav: SectionProps['nav']; onOpenElsewhere: () => void }) {
  const source = useDriveSource()
  const siblings = useSiblings(item, ['raw', 'image'])
  const at = siblings.findIndex((i) => i.id === item.id)
  const [thumbs, setThumbs] = useState<Record<string, string | null>>({})
  // The strip shows the photos near this one. Their blob URLs live in a bounded cache by id (thumbCache.ts), so a step
  // fetches only what is new, and a URL is freed when replaced or pushed out, never while it is shown.
  const near = useMemo(() => siblings.slice(Math.max(0, at - STRIP_REACH), at + STRIP_REACH + 1), [siblings, at])
  useEffect(() => {
    let live = true
    const cache = rawThumbCache(source)
    const want = near.map((s) => [s.id, s.id === item.id ? 'large' : 'small'] as const)
    const held: Record<string, string | null> = {}
    for (const [id, size] of want) { const u = cache.get(id, size); if (u !== undefined) held[id] = u }
    setThumbs((t) => ({ ...t, ...held }))
    for (const [id, size] of want) {
      if (id in held) continue
      source.thumbnail(id, size).then((u) => { cache.put(id, size, u); if (live) setThumbs((t) => ({ ...t, [id]: cache.get(id, size) ?? u })) }, () => {})
    }
    return () => { live = false }
  }, [source, near, item.id])
  const step = useCallback((d: number) => { const n = siblings[at + d]; if (n) nav.go({ section: 'open', id: n.id }) }, [siblings, at, nav])
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey || keyBelongsElsewhere(e)) return
      if (e.key === 'ArrowLeft') { e.preventDefault(); step(-1) } else if (e.key === 'ArrowRight') { e.preventDefault(); step(1) }
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [step])
  return (
    <RawViewer
      src={thumbs[item.id] ?? null} label={`Camera preview · ${rawMaker(item.name)}`} size={`RAW · ${fmtBytes(item.size ?? 0)}`}
      strip={near.map((s) => ({ id: s.id, src: thumbs[s.id] ?? null, current: s.id === item.id }))}
      onPrev={at > 0 ? () => step(-1) : undefined} onNext={at < siblings.length - 1 ? () => step(1) : undefined}
      onPick={(id: string) => nav.go({ section: 'open', id })} onOpenElsewhere={onOpenElsewhere}
    />
  )
}

function ZipView({ bytes }: { bytes: Uint8Array | null }) {
  const [rows, setRows] = useState<{ path: string; size: number | null; depth: number }[] | null>(null)
  const [note, setNote] = useState('')
  useEffect(() => {
    if (!bytes) return
    void import('@beebeeb/shared/drive-ui').then(async ({ readZipListing }) => {
      const l = await readZipListing(new Blob([bytes as BlobPart]))
      if (!l) return setNote('This archive’s list can’t be read here.')
      setRows(l.entries.map((e: { name: string; uncompressedSize: number; isDirectory: boolean }) => ({ path: e.name.replace(/\/$/, '').split('/').pop() ?? e.name, size: e.isDirectory ? null : e.uncompressedSize, depth: Math.max(0, e.name.replace(/\/$/, '').split('/').length - 1) })))
      const total = l.entries.reduce((n: number, e: { uncompressedSize: number }) => n + e.uncompressedSize, 0)
      setNote(`${l.totalEntries} files · ${fmtBytes(total)} unpacked · files inside can’t be opened here`)
    })
  }, [bytes])
  return <ArchiveViewer rows={rows ?? []} note={note || 'Reading the list…'} />
}

/** DW-1503: the folder is the queue. Space plays and pauses, Ctrl+→ and Ctrl+← move through it, a click on a track
 *  plays it; playback carries on when the view is left (player.ts keeps one audio element for the window). */
function AudioView({ item, bytes, nav }: { item: DriveItem; bytes: Uint8Array | null; nav: SectionProps['nav'] }) {
  const [, tick] = useState(0)
  const queue = useSiblings(item, ['audio'])
  const at = Math.max(0, queue.findIndex((i) => i.id === item.id))
  useEffect(() => player.onTime(() => tick((n) => n + 1)), [])
  useEffect(() => { if (bytes && player.playerState().current?.id !== item.id) player.loadTrack(item.id, bytes, { title: titleOf(item.name), artist: '' }, item.mime) }, [bytes, item])
  const go = useCallback((d: number) => { const n = queue[at + d]; if (n) nav.go({ section: 'open', id: n.id }) }, [queue, at, nav])
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (keyBelongsElsewhere(e) || e.altKey || e.metaKey) return
      if (e.key === ' ' && !e.ctrlKey) { e.preventDefault(); player.toggle() }
      else if (e.ctrlKey && e.key === 'ArrowRight') { e.preventDefault(); go(1) }
      else if (e.ctrlKey && e.key === 'ArrowLeft') { e.preventDefault(); go(-1) }
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [go])
  const st = player.playerState()
  const length = Math.round(player.duration()) || 1
  // Only the playing track's length is known (its bytes are here); the others show none rather than a guess.
  const tracks: Track[] = queue.map((q) => ({ id: q.id, title: titleOf(q.name), artist: '', length: q.id === item.id ? length : 0 }))
  return (
    <AudioPlayer
      tracks={tracks} index={at} playing={st.playing && st.current?.id === item.id} elapsed={Math.round(st.elapsed)} fileName={item.name}
      onToggle={player.toggle} onSeek={player.seek} onPrev={() => go(-1)} onNext={() => go(1)} onPick={(i) => { const n = queue[i]; if (n) nav.go({ section: 'open', id: n.id }) }}
    />
  )
}

const titleOf = (name: string) => name.replace(/\.[^.]+$/, '')
const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
/** "Sunday 14:20" within the week, else "8 Nov 2026". */
function whenLabel(iso: string): string {
  const d = new Date(iso)
  const days = (Date.now() - d.getTime()) / 86_400_000
  return days < 7 && days >= 0 ? `${WEEKDAYS[d.getDay()]} ${hhmm(iso)}` : d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' })
}
const RAW_MAKERS: Record<string, string> = { arw: 'Sony ARW', cr2: 'Canon CR2', cr3: 'Canon CR3', nef: 'Nikon NEF', raf: 'Fujifilm RAF', orf: 'Olympus ORF', rw2: 'Panasonic RW2', dng: 'Adobe DNG' }
/** The pill names the camera's own preview, never the RAW itself (DW-1501 Rules). */
const rawMaker = (name: string) => RAW_MAKERS[name.slice(name.lastIndexOf('.') + 1).toLowerCase()] ?? 'RAW'


// ---- The editor: Ctrl+S saves a new encrypted version; leaving with changes asks first ----
/** Ctrl+S saves a new encrypted version and shows the note with "Saved · encrypted · version N" (DW-1523 → DW-1524);
 *  leaving with changes asks first (DW-1525); a newer version saved elsewhere is a choice, never an overwrite (DW-1526). */
function Editor({ item, initial, onSaved, onLeave }: {
  item: DriveItem; initial: { text: string; versionId: string }
  /** Rust wrote the save on this PC; `d.versionId` is Rust's tag of it (the engine uploads it later). */
  onSaved: (d: { text: string; versionId: string }) => void
  /** Back to the viewer; `reload` reads the file again (another device's version replaced this one). */
  onLeave: (reload: boolean) => void
}) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const doc = useRef(initial.text)
  const base = useRef(initial)
  const [dirty, setDirty] = useState(false)
  // Rust's number: the server's version, which a save keeps until the engine has uploaded it (never one made here).
  const version = Number(initial.versionId.replace(/^v(\d+).*$/, '$1')) || 1
  const [saving, setSaving] = useState(false)
  const [leave, setLeave] = useState(false)
  const [conflict, setConflict] = useState<{ theirs: { text: string; versionId: string }; entry: VersionEntry | undefined } | null>(null)
  const busy = useRef(false)
  // Ctrl+F is the editor's own find (DW-1523), from the toolbar too, never Search: leaving would drop the edit.
  const host = useRef<HTMLDivElement>(null)
  const openFind = useCallback(() => openEditorFind(host.current), [])
  useOwnFind(true, openFind)

  const save = useCallback(async (): Promise<boolean> => {
    if (busy.current) return false
    busy.current = true
    setSaving(true)
    try {
      const d = await source.saveText(item.id, doc.current, base.current.versionId)
      base.current = d
      setDirty(false)
      onSaved(d)
      return true
    } catch (e) {
      if (e instanceof DriveSourceError && e.code === 'conflict') {
        try {
          const [theirs, versions] = await Promise.all([source.readText(item.id), source.versions(item.id)])
          setConflict({ theirs, entry: versions.find((v) => v.current) })
        } catch (again) { showToast({ kind: 'failed', text: failedWords(again, 'read the newer version. Your edit is still here') }) }
      } else showToast({ kind: 'failed', text: failedWords(e, 'save. Your edit is still here') })
      return false
    } finally { busy.current = false; setSaving(false) }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, item.id])
  useCommand('save', useCallback(() => { void save() }, [save]))

  useEffect(() => {
    const on = (e: BeforeUnloadEvent) => { if (dirty) e.preventDefault() }
    window.addEventListener('beforeunload', on)
    return () => window.removeEventListener('beforeunload', on)
  }, [dirty])

  const goBack = () => (dirty ? setLeave(true) : onLeave(false))
  const resolve = async (choice: 'mine' | 'theirs' | 'both') => {
    if (!conflict) return
    if (choice === 'mine') {
      // Mine becomes the next version; theirs stays in version history.
      base.current = conflict.theirs
      setConflict(null)
      await save()
    } else if (choice === 'theirs') {
      setConflict(null)
      onLeave(true)
    } else {
      // Mine goes next to it as its own file, under a free name; this file stays theirs.
      try {
        const bytes = new TextEncoder().encode(doc.current)
        const { name } = await createKeptCopy(item.name, 'This PC', () => source.list(item.parentId).then((l) => l.items.map((i) => i.name)), (n) => extras.createFile(item.parentId, n, bytes))
        setConflict(null)
        setDirty(false)
        showToast({ kind: 'done', text: `Your edit is saved as ${name}` })
        onLeave(true)
      } catch (e) {
        setConflict(null)
        showToast({ kind: 'failed', text: failedWords(e, 'save your edit as a copy. It’s still here') })
      }
    }
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <OpenToolbar name={item.name} dirty={dirty} meta={editMeta(dirty, version, saving)} onBack={goBack}>
        <SaveButton onClick={() => void save()} disabled={!dirty || saving} />
        <MoreButton />
      </OpenToolbar>
      <div ref={host} className="contents">
        <TextEditor name={item.name} initialDoc={initial.text} onChange={(d) => { doc.current = d; setDirty(d !== base.current.text) }} onSave={() => void save()} />
      </div>
      {leave && <UnsavedAlert name={item.name} onKeep={() => setLeave(false)} onDiscard={() => onLeave(false)} onSave={() => { setLeave(false); void save() }} />}
      {conflict && (
        <EditConflictDialog
          name={item.name}
          changedOn={`Changed on ${conflict.entry?.deviceName ?? 'another device'} while you were editing`}
          mine={{ label: 'Your edit', when: 'this PC', note: `not saved · ${lineDeltaText(doc.current, conflict.theirs.text)}`, preview: doc.current }}
          theirs={{ label: conflict.entry?.deviceName ?? 'Another device', when: conflict.entry ? hhmm(conflict.entry.createdAt) : '', note: `version ${Number(conflict.theirs.versionId.replace(/^v(\d+).*$/, '$1')) || ''}`, preview: conflict.theirs.text }}
          onKeep={(c) => void resolve(c)} onCancel={() => setConflict(null)}
        />
      )}
    </div>
  )
}
const lineDeltaText = (a: string, b: string) => { const d = lineCount(a) - lineCount(b); const n = Math.abs(d); return d === 0 ? 'edited' : `${d > 0 ? '+' : '−'}${n} line${n === 1 ? '' : 's'}` }

// ---- New file: type, then name; it is made in the folder you are in and opens in the editor ----
function NewFileFlow({ parentId, nav }: { parentId: string | null; nav: SectionProps['nav'] }) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [type, setType] = useState<FileType['id']>('md')
  const [step, setStep] = useState<'type' | 'name'>('type')
  const [name, setName] = useState('Untitled')
  const [ext, setExt] = useState('.md')
  const [taken, setTaken] = useState<string[]>([])
  const [failure, setFailure] = useState<string | null>(null)
  const [making, setMaking] = useState(false)
  useEffect(() => { void source.list(parentId).then((l) => setTaken(l.items.map((i) => i.name.toLowerCase())), () => setTaken([])) }, [source, parentId])
  // Cancel goes back to the folder the file would have been made in (DW-1521 → DW-0401).
  const cancel = () => nav.go({ section: 'files', id: parentId })
  const full = `${name.trim()}${ext}`
  const error = failure ?? (step === 'name' && name !== 'Untitled' ? nameError(name, taken.includes(full.toLowerCase())) : null)
  const create = async () => {
    if (making || nameError(name, taken.includes(full.toLowerCase()))) return
    setMaking(true)
    try {
      // Rust writes the empty file into this folder (encrypted by the engine as it uploads) and answers its id.
      const made = await extras.createFile(parentId, full, new Uint8Array())
      nav.go({ section: 'open', id: made.id, sub: 'edit' })
    } catch (e) {
      setFailure(createFailure(e))
      setMaking(false)
    }
  }
  const pick = () => {
    const t = FILE_TYPES.find((x) => x.id === type)!
    if (t.office) return nav.go({ section: 'open', id: parentId, sub: 'new-document' })
    setExt(t.ext)
    setStep('name')
  }
  return step === 'type'
    ? <NewFileTypeDialog selected={type} onSelect={setType} onContinue={pick} onCancel={cancel} />
    : <NewFileNameDialog markdown={type === 'md'} name={name} ext={ext} error={error} onName={(v) => { setName(v); setFailure(null) }} onExt={(v) => { setExt(v); setFailure(null) }} onCreate={() => void create()} onCancel={cancel} />
}

/** Why a new file was not made, in words: never the host's text. */
export function createFailure(e: unknown): string {
  const code = e instanceof DriveSourceError ? e.code : 'failed'
  if (code === 'name-taken') return 'There’s already a file with that name here.'
  if (code === 'read-only') return 'This folder is read-only. Nothing was made.'
  if (code === 'offline') return 'You’re offline. Nothing was made. Try again when you’re connected.'
  if (code === 'unsupported') return 'Making a file isn’t available in this version of Beebeeb. Nothing was made.'
  return 'Couldn’t make the file. Nothing was made. Try again.'
}

/** DW-1517 live: the free space on the sync folder's disk from B02b's drive_status; absent, the sentence leaves it out.
 *  Without the room, the window's status line says the numbers too, until the file is left. */
function TooLargeLive({ known, ...p }: Omit<Parameters<typeof TooLargeView>[0], 'free'> & { known?: number }) {
  const extras = useDriveExtras()
  // The figure the open already read is passed in; only a too-large read (no figure yet) asks drive_status (#273 finding 5).
  const [free, setFree] = useState<number | undefined>(known)
  useEffect(() => { if (known === undefined) extras.status().then((s) => setFree(s.freeSpaceBytes), () => setFree(undefined)) }, [extras, known])
  useEffect(() => {
    setStatusNote(free !== undefined && free < p.size ? noRoomWords(p.size, free) : null)
    return () => setStatusNote(null)
  }, [free, p.size])
  return <TooLargeView {...p} free={free} />
}
