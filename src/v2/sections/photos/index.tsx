// Photos, live: DriveSource.photos / thumbnail / upload(mode 'photos') / setAvailableOffline / moveToTrash.
// B02b: date taken, camera, dimensions, video length and the uploading flag come from drive_photo_meta (read from the
// copy on this PC; an online-only photo keeps its modified time), the folders from drive_photo_sources. The device
// that took a photo is in no command, so the device pills and backup lines stay absent: never invented.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useDriveSource, type PhotoEntry } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { loadAcross, saveAcross, ACROSS, cellMeta, idOf, isVideo, groupByDay, type Across, type PhotoCell } from './model'
import { useDriveExtras, type PhotoMeta, type PhotoSource } from '../../source/extras'
import { PhotosView } from './PhotosView'
import { PhotoViewer } from './Viewer'
import { DetailsDialog, ShareDialog, SourcesDialog } from './Dialogs'

export default function Section(_props: SectionProps) {
  const source = useDriveSource()
  const [entries, setEntries] = useState<PhotoEntry[] | null>(null)
  const [across, setAcrossState] = useState<Across>(loadAcross)
  const [selecting, setSelecting] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [viewing, setViewing] = useState<string | null>(null)
  const [mode, setMode] = useState<'preview' | 'original'>('preview')
  const [dialog, setDialog] = useState<'sources' | 'share' | 'details' | null>(null)
  const [kept, setKept] = useState<Set<string>>(new Set())
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
    const ids = all.map((e) => e.item.id)
    const pages = await Promise.all(Array.from({ length: Math.ceil(ids.length / 200) }, (_, i) => extras.photoMeta(ids.slice(i * 200, i * 200 + 200)).catch(() => ({}))))
    setMetas(Object.assign({}, ...pages))
  }, [source, extras])

  useEffect(() => {
    load().catch(() => setEntries([]))
    return source.subscribe((e) => { if (e.kind === 'changed') load().catch(() => {}) })
  }, [source, load])

  const setAcross = (n: Across) => { setAcrossState(n); saveAcross(n) }
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (!e.ctrlKey || viewing) return
      const i = ACROSS.indexOf(across)
      if (e.key === '+' || e.key === '=') { e.preventDefault(); setAcross(ACROSS[Math.max(0, i - 1)]) }
      else if (e.key === '-') { e.preventDefault(); setAcross(ACROSS[Math.min(ACROSS.length - 1, i + 1)]) }
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [across, viewing])

  // Date taken over modified time where the file says it.
  const cells: PhotoCell[] = useMemo(() => (entries ?? []).map((entry) => {
    const m = metas[entry.item.id]
    return { entry: m?.takenAt ? { ...entry, takenAt: m.takenAt } : entry, meta: cellMeta(m) }
  }), [entries, metas])
  const ordered = useMemo(() => groupByDay(cells).flatMap((d) => d.cells), [cells])
  const totals = entries ? { photos: cells.filter((c) => !isVideo(c)).length, videos: cells.filter(isVideo).length, bytes: cells.reduce((n, c) => n + (c.entry.item.size ?? 0), 0) } : null
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
    } catch { showToast({ kind: 'failed', text: 'Couldn’t move to Trash' }) }
  }
  const keep = async (ids: string[]) => {
    try {
      await Promise.all(ids.map((id) => source.setAvailableOffline(id, !kept.has(id) || ids.length > 1)))
      setKept((k) => { const n = new Set(k); ids.forEach((id) => (ids.length === 1 && n.has(id) ? n.delete(id) : n.add(id))); return n })
    } catch { showToast({ kind: 'failed', text: 'Couldn’t keep it on this PC' }) }
  }
  const upload = () => { source.upload(null, { kind: 'pick', mode: 'photos' }).catch(() => showToast({ kind: 'failed', text: 'Couldn’t start the upload' })) }

  const current = index >= 0 ? ordered[index] : null
  const chosen = ordered.find((c) => selected.has(idOf(c))) ?? current
  const dialogs = (
    <>
      {dialog === 'sources' && <SourcesDialog devices={[]} folders={sources.map((f) => ({ name: f.name, items: f.count, note: f.path || undefined, on: true }))} onAddFolder={() => showToast({ kind: 'failed', text: 'Choosing folders for Photos isn’t available in this build yet' })} onCancel={() => setDialog(null)} onDone={() => setDialog(null)} />}
      {dialog === 'share' && chosen && <ShareDialog cell={chosen} onClose={() => setDialog(null)} onCopyLink={() => { showToast({ kind: 'done', text: 'Link copied' }); setDialog(null) }} onSendCopy={() => setDialog(null)} />}
      {dialog === 'details' && chosen && <DetailsDialog cell={chosen} now={now} onClose={() => setDialog(null)} />}
    </>
  )

  if (current) {
    return (
      <>
        <PhotoViewer
          cells={ordered} index={index} now={now} mode={mode} kept={kept.has(idOf(current))}
          onBack={() => { setViewing(null); setMode('preview') }}
          onStep={(d) => { const n = ordered[index + d]; if (n) setViewing(idOf(n)) }}
          onJump={(i) => setViewing(idOf(ordered[i]))}
          onMode={(m) => { setMode(m); if (m === 'original') source.open(idOf(current)).catch(() => showToast({ kind: 'failed', text: 'Couldn’t open the original' })) }}
          onMore={(a) => { if (a === 'duplicate') source.duplicate([idOf(current)]).then(load, () => showToast({ kind: 'failed', text: 'Couldn’t duplicate it' })) }}
          onShare={() => setDialog('share')} onDetails={() => setDialog('details')}
          onOffline={() => keep([idOf(current)])} onTrash={() => trash([idOf(current)])} onOpen={() => { source.open(idOf(current)).catch(() => {}) }}
        />
        {dialogs}
      </>
    )
  }
  return (
    <>
      <PhotosView
        cells={cells} now={now} across={across} devices={[]} device={null} totals={totals} backups={[]}
        selecting={selecting} selected={selected} loading={entries === null} canUpload={source.capabilities.nativePickers}
        onOpen={(id) => setViewing(id)} onToggle={toggle} onAcross={setAcross}
        onSources={() => { extras.photoSources().then(setSources, () => setSources([])); setDialog('sources') }} onSelect={() => { setSelecting((s) => !s); setSelected(new Set()) }}
        onSelectAll={() => setSelected(new Set(cells.map(idOf)))} onDone={() => { setSelecting(false); setSelected(new Set()) }}
        onUpload={upload} onShare={() => setDialog('share')} onOffline={() => keep([...selected])}
        onMove={() => showToast({ kind: 'failed', text: 'Choose where to move them in Files' })} onTrash={() => trash([...selected])}
      />
      {dialogs}
    </>
  )
}
