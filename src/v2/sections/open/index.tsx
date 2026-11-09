// Open: one file in the Drive window's content area, under the window's toolbar (DW-1501..1527). Office is B08's.
// Reads and saves only through useDriveSource(). The route is `section=open&id=<file>[&sub=edit]`; `sub=new-file&id=<folder>`.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { DriveSourceError, MAX_EDITABLE_BYTES, insertBeforeExtension, useDriveSource, type DriveItem, type DriveSource, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { useCommand } from '../../shortcuts'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { setStatusNote } from '../../chrome/statusNote'
import { failedWords } from '../photos/Dialogs'
import { ShareHost, VersionsHost } from './hosts'
import { DetailsRail, FindBar, MoreButton, NewerBanner, OpenToolbar, ToolbarButton } from './Chrome'
import { EditConflictDialog, editMeta, NewFileNameDialog, NewFileTypeDialog, nameError, FILE_TYPES, SaveButton, SavedMessage, TextEditor, UnsavedAlert, type FileType } from './Editing'
import { DecryptFailedView, DecryptingView, OpenInAppNote, TooLargeView } from './States'
import { useDriveExtras } from '../../source/extras'
import { ArchiveViewer, AudioPlayer, Centered, CodeViewer, CsvViewer, ImageViewer, MarkdownViewer, PdfZoom, RawViewer, type Track } from './Viewers'
import { fmtBytes, hhmm, languageLabel, lineCount, READ_ONLY_LINES, viewerKind } from './model'
import * as player from './player'
import { isOfficeFile, NewDocumentFlow, OfficeEditor } from './office'

type Loaded =
  | { kind: 'text'; text: string; versionId: string }
  | { kind: 'bytes'; bytes: Uint8Array; versionId: string }
  | { kind: 'none' }

/** `loading` carries the item once its details are in, so Decrypting names it and Cancel knows its folder (DW-1516). */
type Phase = { name: 'loading'; item: DriveItem | null } | { name: 'ready'; item: DriveItem; data: Loaded } | { name: 'too-large'; item: DriveItem } | { name: 'failed'; item: DriveItem | null }

const TEXTUAL = new Set(['code', 'text', 'markdown', 'csv'])

export default function Section({ nav }: SectionProps) {
  const { id, sub } = nav.route
  if (sub === 'new-file') return <NewFileFlow parentId={id} nav={nav} />
  if (sub === 'new-document') return <NewDocumentFlow parentId={id} nav={nav} />
  if (sub === 'office' && id) return <OfficeEditor key={id} target={{ kind: 'file', id }} nav={nav} />
  if (!id) return <Centered><p className="m-0 text-[13px] text-ink-3">Nothing is open.</p></Centered>
  return <OpenFile key={id} id={id} editing={sub === 'edit'} nav={nav} />
}

function useLoad(id: string) {
  const source = useDriveSource()
  const [phase, setPhase] = useState<Phase>({ name: 'loading', item: null })
  const [tries, setTries] = useState(0)
  useEffect(() => {
    let live = true
    let known: DriveItem | null = null
    setPhase((p) => ({ name: 'loading', item: 'item' in p ? p.item : null }))
    ;(async () => {
      try {
        const { item } = await source.details(id)
        known = item
        if (live) setPhase({ name: 'loading', item })
        const kind = viewerKind(item.name)
        const data: Loaded = TEXTUAL.has(kind) ? { kind: 'text', ...(await source.readText(id)) } : kind === 'other' ? { kind: 'none' } : { kind: 'bytes', ...(await source.readBytes(id)) }
        if (live) setPhase({ name: 'ready', item, data })
      } catch (e) {
        if (!live) return
        if (e instanceof DriveSourceError && e.code === 'quota') {
          const { item } = await source.details(id).catch(() => ({ item: null }))
          if (item) return setPhase({ name: 'too-large', item })
        }
        setPhase({ name: 'failed', item: known })
      }
    })()
    return () => { live = false }
  }, [source, id, tries])
  return { phase, retry: () => setTries((n) => n + 1), setPhase }
}

/**
 * A newer version arriving while the file is open is announced, never swapped in under the reader (DW-1519). The
 * version that was current when the file was read is the baseline; a later current version is newer, unless this
 * window saved it (its own upload is not "a newer version arrived"). Compared by the server's version ids, never by
 * the read's content tag (a different shape: `v3-…`).
 */
function useNewer(source: DriveSource, id: string, ready: boolean) {
  const [newer, setNewer] = useState<VersionEntry | null>(null)
  const base = useRef<string | null>(null)
  const ours = useRef(false)
  const rebase = useCallback(() => { source.versions(id).then((v) => { base.current = v.find((x) => x.current)?.id ?? null }, () => {}) }, [source, id])
  useEffect(() => { if (ready) rebase() }, [ready, rebase])
  useEffect(() => source.subscribe((e) => {
    if (e.kind !== 'item' || e.item.id !== id) return
    void source.versions(id).then((v) => {
      const cur = v.find((x) => x.current)
      if (!cur || cur.id === base.current) return
      if (ours.current) { ours.current = false; base.current = cur.id; return }
      setNewer(cur)
    }, () => {})
  }), [source, id])
  return {
    newer,
    dismiss: () => setNewer(null),
    /** This window saved a new version: that is not news, and what was announced before it is settled. */
    savedHere: () => { ours.current = true; setNewer(null); rebase() },
  }
}

function OpenFile({ id, editing, nav }: { id: string; editing: boolean; nav: SectionProps['nav'] }) {
  const source = useDriveSource()
  const { phase, retry, setPhase } = useLoad(id)
  const { newer, dismiss: dismissNewer, savedHere } = useNewer(source, id, phase.name === 'ready')
  const [versionsOpen, setVersionsOpen] = useState(false)
  const [panel, setPanel] = useState(false)
  // DW-1524: after a save the note shows rendered, with "Saved · encrypted · version N".
  const [savedVersion, setSavedVersion] = useState<number | null>(null)
  // The window's status line says the save too, until the file is left or edited again (DW-1524).
  useEffect(() => () => setStatusNote(null), [])
  useEffect(() => { if (editing || savedVersion === null) setStatusNote(null) }, [editing, savedVersion])
  useCommand('toggle-details-panel', useCallback(() => setPanel((p) => !p), []))
  // "Back to the folder" (the toolbar's back): the folder the file is in, whatever led here (DW-1503, DW-1507).
  const toFolder = (item: DriveItem | null) => (item ? nav.go({ section: 'files', id: item.parentId }) : nav.back())

  if (phase.name === 'loading') {
    // Cancel stops the read; nothing half-decrypted is kept (Rust writes nothing until the read is whole).
    return <Frame name={phase.item?.name ?? ''} onBack={() => toFolder(phase.item)}><DecryptingView name={phase.item?.name ?? ''} onCancel={() => toFolder(phase.item)} /></Frame>
  }
  if (phase.name === 'failed') return <Frame name={phase.item?.name ?? ''} onBack={() => toFolder(phase.item)}><DecryptFailedView onRetry={retry} onReport={() => nav.go({ section: 'settings', sub: 'report' })} /></Frame>
  if (phase.name === 'too-large') {
    return <Frame name={phase.item.name} onBack={() => toFolder(phase.item)}><TooLargeLive name={phase.item.name} size={phase.item.size ?? 0} onFreeUp={() => nav.go({ section: 'settings', sub: 'storage' })} onDownloadTo={() => showToast({ kind: 'failed', text: 'Saving a decrypted copy elsewhere isn’t available in this version of Beebeeb.' })} /></Frame>
  }
  // Office files open in the editor by default (DW-1562); CSV keeps its table viewer.
  if (isOfficeFile(phase.item) && viewerKind(phase.item.name) !== 'csv') return <OfficeEditor target={{ kind: 'file', id }} nav={nav} />
  const { item, data } = phase
  const showNewer = () => { dismissNewer(); setSavedVersion(null); retry() }
  const banner = newer && !editing
    ? <NewerBanner text={`A newer version arrived from ${newer.deviceName ?? 'another device'}, ${hhmm(newer.createdAt)}.`} onShow={showNewer} />
    : savedVersion !== null && !editing ? <SavedMessage version={savedVersion} onVersions={() => setVersionsOpen(true)} /> : null
  const aside = panel ? <DetailsAside id={id} onClose={() => setPanel(false)} /> : null
  const saved = (d: { text: string; versionId: string }, version: number) => {
    setPhase({ name: 'ready', item, data: { kind: 'text', ...d } })
    setSavedVersion(version)
    savedHere()
    setStatusNote(`Saved · encrypted · version ${version} · ${hhmm(new Date().toISOString())}`)
    nav.go({ section: 'open', id: item.id, sub: null })
  }
  const body = editing && data.kind === 'text'
    ? <Editor item={item} initial={data} onSaved={saved} onLeave={(reload) => { nav.go({ section: 'open', id: item.id, sub: null }); if (reload) { dismissNewer(); retry() } }} />
    : <Viewer item={item} data={data} nav={nav} banner={banner} onBack={() => toFolder(item)} onDetails={() => setPanel((p) => !p)} onEdit={() => setSavedVersion(null)} />
  return (
    <div className="flex min-h-0 flex-1">
      {body}{aside}
      {versionsOpen && <VersionsHost item={item} onClose={() => setVersionsOpen(false)} />}
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
  const [zoom, setZoom] = useState(1)
  const [find, setFind] = useState<string | null>(null)
  const [share, setShare] = useState(false)
  const bytes = data.kind === 'bytes' ? data.bytes : null
  const url = useBlobUrl(bytes, item.mime ?? 'application/octet-stream')
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
  const zoomBy = (d: number) => setZoom((z) => Math.min(3, Math.max(0.5, +(z + d).toFixed(2))))
  // Ctrl+F finds in the document; Ctrl+Plus and Ctrl+− zoom the page (DW-1505).
  useCommand('find', useCallback(() => { if (kind === 'pdf') setFind('') }, [kind]))
  useCommand('zoom-in', useCallback(() => { if (kind === 'pdf') zoomBy(0.25) }, [kind]))
  useCommand('zoom-out', useCallback(() => { if (kind === 'pdf') zoomBy(-0.25) }, [kind]))
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
      {kind === 'pdf' && <><PdfZoom zoom={zoom} onZoom={setZoom} /><ToolbarButton label="Find" shortcut="Ctrl+F" onClick={() => setFind('')} /><ToolbarButton label="Share" onClick={() => setShare(true)} /></>}
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
  else if (kind === 'pdf') body = url ? <div className="flex min-h-0 flex-1 flex-col">{find !== null && <FindBar query={find} onQuery={setFind} onDone={() => setFind(null)} message="Find works in the page view: click the page, then Ctrl+F" />}<PdfFrame url={url} zoom={zoom} /></div> : null
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

/** The page view: WebView2's own PDF viewer draws the pages, their thumbnails and its find; the zoom is passed to it. */
function PdfFrame({ url, zoom }: { url: string; zoom: number }) {
  return <iframe key={zoom} src={`${url}#zoom=${Math.round(zoom * 100)}`} sandbox="allow-popups allow-scripts" title="PDF preview" className="block h-full min-h-0 w-full flex-1 border-0" />
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

/** DW-1501: a RAW shows the preview the camera stored in it (the item's thumbnail), and says so; ← → step through the folder. */
function RawView({ item, nav, onOpenElsewhere }: { item: DriveItem; nav: SectionProps['nav']; onOpenElsewhere: () => void }) {
  const source = useDriveSource()
  const siblings = useSiblings(item, ['raw', 'image'])
  const at = siblings.findIndex((i) => i.id === item.id)
  const [thumbs, setThumbs] = useState<Record<string, string | null>>({})
  useEffect(() => {
    let live = true
    for (const s of siblings) source.thumbnail(s.id, s.id === item.id ? 'large' : 'small').then((u) => { if (live) setThumbs((t) => ({ ...t, [s.id]: u })) }, () => {})
    return () => { live = false }
  }, [source, siblings, item.id])
  const step = useCallback((d: number) => { const n = siblings[at + d]; if (n) nav.go({ section: 'open', id: n.id }) }, [siblings, at, nav])
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey || isTyping(e.target)) return
      if (e.key === 'ArrowLeft') { e.preventDefault(); step(-1) } else if (e.key === 'ArrowRight') { e.preventDefault(); step(1) }
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [step])
  return (
    <RawViewer
      src={thumbs[item.id] ?? null} label={`Camera preview · ${rawMaker(item.name)}`} size={`RAW · ${fmtBytes(item.size ?? 0)}`}
      strip={siblings.map((s) => ({ id: s.id, src: thumbs[s.id] ?? null, current: s.id === item.id }))}
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
      if (isTyping(e.target) || e.altKey || e.metaKey) return
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
const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))
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
  onSaved: (d: { text: string; versionId: string }, version: number) => void
  /** Back to the viewer; `reload` reads the file again (another device's version replaced this one). */
  onLeave: (reload: boolean) => void
}) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const doc = useRef(initial.text)
  const base = useRef(initial)
  const [dirty, setDirty] = useState(false)
  const version = Number(initial.versionId.replace(/^v(\d+).*$/, '$1')) || 1
  const [saving, setSaving] = useState(false)
  const [leave, setLeave] = useState(false)
  const [conflict, setConflict] = useState<{ theirs: { text: string; versionId: string }; entry: VersionEntry | undefined } | null>(null)
  const busy = useRef(false)

  const save = useCallback(async (): Promise<boolean> => {
    if (busy.current) return false
    busy.current = true
    setSaving(true)
    try {
      const d = await source.saveText(item.id, doc.current, base.current.versionId)
      base.current = d
      setDirty(false)
      // Rust's tag keeps the server's number until the engine has uploaded it; the save is the next version either way.
      onSaved(d, Math.max(Number(d.versionId.replace(/^v(\d+).*$/, '$1')) || 0, version + 1))
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
      // Mine goes next to it as its own file; this file stays theirs.
      const name = insertBeforeExtension(item.name, ' (This PC)')
      try {
        await extras.createFile(item.parentId, name, new TextEncoder().encode(doc.current))
        setConflict(null)
        setDirty(false)
        showToast({ kind: 'done', text: `Your edit is saved as ${name}` })
        onLeave(true)
      } catch (e) {
        setConflict(null)
        showToast({ kind: 'failed', text: failedWords(e, `save your edit as ${name}. It’s still here`) })
      }
    }
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <OpenToolbar name={item.name} dirty={dirty} meta={editMeta(dirty, version, saving)} onBack={goBack}>
        <SaveButton onClick={() => void save()} disabled={!dirty || saving} />
        <MoreButton />
      </OpenToolbar>
      <TextEditor name={item.name} initialDoc={initial.text} onChange={(d) => { doc.current = d; setDirty(d !== base.current.text) }} onSave={() => void save()} />
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

/** DW-1517 live: the free space on the sync folder's disk from B02b's drive_status; absent, the sentence leaves it out. */
function TooLargeLive(p: Omit<Parameters<typeof TooLargeView>[0], 'free'>) {
  const extras = useDriveExtras()
  const [free, setFree] = useState<number | undefined>(undefined)
  useEffect(() => { extras.status().then((s) => setFree(s.freeSpaceBytes), () => setFree(undefined)) }, [extras])
  // The window's own status line stays (V2App draws it); the sentence carries the free-space figure.
  return <TooLargeView {...p} free={free} />
}
