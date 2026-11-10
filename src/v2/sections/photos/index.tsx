// Photos, live: DriveSource.photos / thumbnail / upload(mode 'photos') / setAvailableOffline / moveToTrash.
// B02b: date taken, camera, dimensions, video length and the uploading flag come from drive_photo_meta (read from the
// copy on this PC; an online-only photo keeps its modified time), the folders from drive_photo_sources. M3d: the
// device a photo came from is its backup's device folder (drive_photo_meta `device`), so the pills and Photo sources'
// devices are live; the devices' backup progress is in no command, so that line stays absent: never invented.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useDriveSource, type PhotoEntry } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { copyShareLink } from '../files/shareLink'
import {
  loadAcross, saveAcross, ACROSS, cellMeta, devicesOf, folderKey, groupByDay, idOf, loadHidden, saveHidden, shown, totalsOf, type Across, type PhotoCell,
} from './model'
import { useDriveExtras, type PhotoMeta, type PhotoSource, type TransferRow } from '../../source/extras'
import { noteListedPhotos, PhotosView } from './PhotosView'
import { PhotoViewer } from './Viewer'
import { DetailsDialog, failedWords, PhotoMove, RenameDialog, ShareDialog, SourcesDialog, type FolderChoice, type SourceChoice } from './Dialogs'

type Dialog = 'sources' | 'share' | 'details' | 'move' | 'rename'

export default function Section(_props: SectionProps) {
  const source = useDriveSource()
  const [entries, setEntries] = useState<PhotoEntry[] | null>(null)
  const [across, setAcrossState] = useState<Across>(loadAcross)
  const [selecting, setSelecting] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [viewing, setViewing] = useState<string | null>(null)
  const [mode, setMode] = useState<'preview' | 'original'>('preview')
  // DW-0605b: the original is still being read; `done`/`total` are only what the source reports in its transfer rows.
  const [opening, setOpening] = useState<{ id: string; done?: number; total?: number } | null>(null)
  const [dialog, setDialog] = useState<Dialog | null>(null)
  const [kept, setKept] = useState<Set<string>>(new Set())
  // DW-0608: the message after Offline, for the photo it was asked on.
  const [keeping, setKeeping] = useState<string | null>(null)
  const [device, setDevice] = useState<string | null>(null)
  const [hidden, setHiddenState] = useState<Set<string>>(loadHidden)
  const now = useMemo(() => new Date(), [])
  const extras = useDriveExtras()
  const [metas, setMetas] = useState<Record<string, PhotoMeta>>({})
  const [sources, setSources] = useState<PhotoSource[]>([])

  const load = useCallback(async () => {
    const all: PhotoEntry[] = []
    let cursor: string | undefined
    for (let page = 0; page < 50; page += 1) {
      const r = await source.photos(cursor)
      all.push(...r.entries)
      if (!r.next) break
      cursor = r.next
    }
    setEntries(all)
    // What is already kept on this PC reads "On this PC" (DW-0608).
    setKept(new Set(all.filter((e) => e.item.local === 'available-offline').map((e) => e.item.id)))
    const ids = all.map((e) => e.item.id)
    const pages = await Promise.all(Array.from({ length: Math.ceil(ids.length / 200) }, (_, i) => extras.photoMeta(ids.slice(i * 200, i * 200 + 200)).catch(() => ({}))))
    setMetas(Object.assign({}, ...pages))
  }, [source, extras])

  useEffect(() => {
    load().catch(() => setEntries([]))
    return source.subscribe((e) => { if (e.kind === 'changed') load().catch(() => {}) })
  }, [source, load])

  // The bytes the download row reports while the original is opening; without both numbers the bar stays absent.
  const openingId = opening?.id ?? null
  useEffect(() => {
    if (!openingId) return
    let live = true
    const take = (rows: TransferRow[]) => {
      const row = rows.find((r) => r.direction === 'down' && (r.fileId === openingId || r.id === openingId))
      const total = row?.total ?? 0
      if (!row || total <= 0) return
      setOpening((o) => (o && o.id === openingId ? { ...o, done: Math.max(0, Math.min(row.bytes, total)), total } : o))
    }
    extras.transfers().then((rows) => { if (live && Array.isArray(rows)) take(rows) }, () => {})
    const stop = extras.subscribeTransfers((rows) => { if (live && Array.isArray(rows)) take(rows) })
    return () => { live = false; stop() }
  }, [openingId, extras])

  const setAcross = (n: Across) => { setAcrossState(n); saveAcross(n) }
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (viewing || dialog) return
      // Esc ends selecting (DW-0602).
      if (e.key === 'Escape' && selecting) { e.preventDefault(); setSelecting(false); setSelected(new Set()); return }
      if (!e.ctrlKey) return
      const i = ACROSS.indexOf(across)
      if (e.key === '+' || e.key === '=') { e.preventDefault(); setAcross(ACROSS[Math.max(0, i - 1)]) }
      else if (e.key === '-') { e.preventDefault(); setAcross(ACROSS[Math.min(ACROSS.length - 1, i + 1)]) }
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [across, viewing, dialog, selecting])

  // Date taken over modified time where the file says it.
  const all: PhotoCell[] = useMemo(() => (entries ?? []).map((entry) => {
    const m = metas[entry.item.id]
    return { entry: m?.takenAt ? { ...entry, takenAt: m.takenAt } : entry, meta: cellMeta(m) }
  }), [entries, metas])
  // Photo sources' choices apply first; the pills filter what is left (DW-0601, DW-0604).
  const cells = useMemo(() => shown(all, hidden), [all, hidden])
  const devices = useMemo(() => devicesOf(cells), [cells])
  const visible = useMemo(() => (device ? cells.filter((c) => c.meta?.device === device) : cells), [cells, device])
  const ordered = useMemo(() => groupByDay(visible).flatMap((d) => d.cells), [visible])
  const totals = entries ? totalsOf(visible) : null
  const index = viewing ? ordered.findIndex((c) => idOf(c) === viewing) : -1

  const toggle = (id: string, on?: boolean) => setSelected((s) => {
    const next = new Set(s)
    if (on === true || (on === undefined && !next.has(id))) next.add(id)
    else next.delete(id)
    if (next.size > 0) setSelecting(true)
    return next
  })

  const trash = async (ids: string[]) => {
    try {
      const { undoToken } = await source.moveToTrash(ids)
      showToast({ kind: 'undo', text: 'Moved to Trash', onAction: () => { source.undoTrash(undoToken).then(load, () => showToast({ kind: 'failed', text: 'Couldn’t bring it back' })) } })
      setSelected(new Set()); setSelecting(false)
      if (viewing) {
        const next = ordered[index + 1] ?? ordered[index - 1]
        setViewing(next ? idOf(next) : null)
      }
      await load()
    } catch (e) { showToast({ kind: 'failed', text: failedWords(e, 'move it to Trash') }) }
  }
  /** Keep the originals on this PC (Offline), or stop keeping one (On this PC › Remove from this PC). */
  const keep = async (ids: string[], on: boolean) => {
    try {
      await Promise.all(ids.map((id) => source.setAvailableOffline(id, on)))
      setKept((k) => { const n = new Set(k); ids.forEach((id) => (on ? n.add(id) : n.delete(id))); return n })
      setKeeping(on && ids.length === 1 ? ids[0] : null)
      if (!on) showToast({ kind: 'done', text: 'Removed from this PC. It stays in Beebeeb.' })
      else if (ids.length > 1) showToast({ kind: 'done', text: `Keeping ${ids.length} originals on this PC` })
    } catch (e) { showToast({ kind: 'failed', text: failedWords(e, on ? 'keep it on this PC' : 'remove it from this PC') }) }
  }
  const upload = () => { source.upload(null, { kind: 'pick', mode: 'photos' }).catch((e: unknown) => showToast({ kind: 'failed', text: failedWords(e, 'start the upload') })) }
  const openSources = () => { extras.photoSources().then(setSources, () => setSources([])); setDialog('sources') }

  // Photo sources (DW-0604): the devices from the photos' backups, the folders from Files that hold photos. A folder
  // that holds a device's backup is that device, not a folder from Files.
  const backupFolders = new Set(all.filter((c) => c.meta?.device).map((c) => c.entry.item.parentId))
  const deviceChoices: SourceChoice[] = devicesOf(all).map((d) => ({ name: d.name, photos: d.photos, on: !hidden.has(`device:${d.name}`) }))
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
  const folderChoices: FolderChoice[] = sources.filter((f) => !backupFolders.has(f.id)).map((f) => ({
    id: f.id ?? undefined, name: f.name, items: f.count, note: f.path && f.path !== f.name ? f.path : undefined, on: !hidden.has(folderKey(f.id)),
  }))
  const applySources = (d: SourceChoice[], f: FolderChoice[]) => {
    const next = new Set<string>()
    d.forEach((x) => { if (!x.on) next.add(`device:${x.name}`) })
    f.forEach((x) => { if (!x.on) next.add(folderKey(x.id)) })
    setHiddenState(next)
    saveHidden(next)
    if (device && next.has(`device:${device}`)) setDevice(null)
    setDialog(null)
  }

  noteListedPhotos(ordered.map(idOf))
  const current = index >= 0 ? ordered[index] : null
  const chosen = ordered.find((c) => selected.has(idOf(c))) ?? current
  const dialogs = (
    <>
      {dialog === 'sources' && (
        <SourcesDialog
          devices={deviceChoices} folders={folderChoices}
          onAddFolder={() => showToast({ kind: 'failed', text: 'Choosing folders for Photos isn’t available in this version of Beebeeb.' })}
          onCancel={() => setDialog(null)} onDone={applySources}
        />
      )}
      {dialog === 'share' && chosen && (
        <ShareDialog
          cell={chosen} onClose={() => setDialog(null)}
          // Copy link keeps it encrypted: Rust makes the link and puts it on the clipboard (as Files' Copy link, DW-0407).
          onCopyLink={() => { const ids = selecting && selected.size > 0 ? [...selected] : [idOf(chosen)]; setDialog(null); void copyShareLink(ids, { expires: 'In 30 days', password: null }, showToast) }}
          onSendCopy={() => { setDialog(null); showToast({ kind: 'failed', text: 'Sending a copy through the Windows Share menu isn’t available in this version of Beebeeb. Copy link keeps it encrypted.' }) }}
        />
      )}
      {dialog === 'details' && chosen && <DetailsDialog cell={chosen} now={now} onClose={() => setDialog(null)} />}
      {dialog === 'move' && (current || selected.size > 0) && (
        <PhotoMove
          items={current ? [current.entry.item] : visible.filter((c) => selected.has(idOf(c))).map((c) => c.entry.item)}
          onDone={(moved) => { setDialog(null); if (moved) { setSelected(new Set()); setSelecting(false); void load() } }}
        />
      )}
      {dialog === 'rename' && current && <RenameDialog item={current.entry.item} siblings={all.map((c) => c.entry.item).filter((i) => i.parentId === current.entry.item.parentId)} onDone={(renamed) => { setDialog(null); if (renamed) void load() }} />}
    </>
  )

  if (current) {
    const id = idOf(current)
    return (
      <>
        <PhotoViewer
          cells={ordered} index={index} now={now} mode={mode} kept={kept.has(id)} keeping={keeping === id && kept.has(id)}
          opening={opening?.id === id}
          progress={opening?.id === id && opening.done !== undefined && opening.total !== undefined && opening.total > 0 ? { done: opening.done, total: opening.total } : null}
          onBack={() => { if (dialog) return; setViewing(null); setMode('preview'); setKeeping(null); setOpening(null) }}
          onStep={(d) => { if (dialog) return; const n = ordered[index + d]; if (n) { setViewing(idOf(n)); setKeeping(null); setOpening(null) } }}
          onJump={(i) => { setViewing(idOf(ordered[i])); setKeeping(null); setOpening(null) }}
          onMode={(m) => {
            if (m === 'preview') { setMode('preview'); setOpening(null); return }
            setMode('original')
            setOpening({ id })
            source.open(id).then(
              () => setOpening((o) => (o && o.id === id ? null : o)),
              (e: unknown) => {
                setOpening((o) => (o && o.id === id ? null : o))
                setMode('preview')
                showToast({ kind: 'failed', text: failedWords(e, 'open the original') })
              },
            )
          }}
          onMore={(a) => {
            if (a === 'duplicate') source.duplicate([id]).then(() => { showToast({ kind: 'done', text: 'Duplicated' }); return load() }, (e: unknown) => showToast({ kind: 'failed', text: failedWords(e, 'duplicate it') }))
            else if (a === 'move') setDialog('move')
            else if (a === 'rename') setDialog('rename')
            // A save panel that writes a plain decrypted copy needs a command this version does not have: say so.
            else showToast({ kind: 'failed', text: 'Saving a copy to this PC isn’t available in this version of Beebeeb.' })
          }}
          onShare={() => setDialog('share')} onDetails={() => setDialog('details')}
          onOffline={() => keep([id], true)} onRemoveLocal={() => keep([id], false)} onTrash={() => trash([id])}
          onOpen={() => { source.open(id).catch((e: unknown) => showToast({ kind: 'failed', text: failedWords(e, 'open it') })) }}
        />
        {dialogs}
      </>
    )
  }
  return (
    <>
      <PhotosView
        cells={visible} now={now} across={across} devices={devices.map((d) => d.name)} device={device} totals={totals} backups={[]}
        selecting={selecting} selected={selected} loading={entries === null} canUpload={source.capabilities.nativePickers}
        onDevice={setDevice} onOpen={(id) => setViewing(id)} onToggle={toggle} onAcross={setAcross}
        onSources={openSources} onSelect={() => { setSelecting((s) => !s); setSelected(new Set()) }}
        onSelectAll={() => setSelected(new Set(visible.map(idOf)))} onDone={() => { setSelecting(false); setSelected(new Set()) }}
        onUpload={upload} onShare={() => setDialog('share')} onOffline={() => keep([...selected], true)}
        onMove={() => setDialog('move')} onTrash={() => trash([...selected])}
      />
      {dialogs}
    </>
  )
}
