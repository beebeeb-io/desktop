// Open: one file in the Drive window's content area, under the window's toolbar (DW-1501..1527). Office is B08's.
// Reads and saves only through useDriveSource(). The route is `section=open&id=<file>[&sub=edit]`; `sub=new-file&id=<folder>`.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { DriveSourceError, MAX_EDITABLE_BYTES, insertBeforeExtension, useDriveSource, type DriveItem, type DriveSource, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { useCommand } from '../../shortcuts'
import type { SectionProps } from '../types'
import { DetailsRail, FindBar, MoreButton, NewerBanner, OpenToolbar, ToolbarButton } from './Chrome'
import { EditConflictDialog, editMeta, NewFileNameDialog, NewFileTypeDialog, nameError, FILE_TYPES, SaveButton, SavedMessage, TextEditor, UnsavedAlert, type FileType } from './Editing'
import { DecryptFailedView, DecryptingView, OpenInAppNote, TooLargeView } from './States'
import { ArchiveViewer, AudioPlayer, Centered, CodeViewer, CsvViewer, ImageViewer, MarkdownViewer, PdfZoom, type Track } from './Viewers'
import { PdfPreview } from '@beebeeb/shared/drive-ui'
import { fmtBytes, hhmm, languageLabel, lineCount, parseTable, READ_ONLY_LINES, viewerKind } from './model'
import * as player from './player'

type Loaded =
  | { kind: 'text'; text: string; versionId: string }
  | { kind: 'bytes'; bytes: Uint8Array; versionId: string }
  | { kind: 'none' }

type Phase = { name: 'loading' } | { name: 'ready'; item: DriveItem; data: Loaded } | { name: 'too-large'; item: DriveItem } | { name: 'failed' }

const TEXTUAL = new Set(['code', 'text', 'markdown', 'csv'])

export default function Section({ nav }: SectionProps) {
  const { id, sub } = nav.route
  if (sub === 'new-file') return <NewFileFlow parentId={id} nav={nav} />
  if (!id) return <Centered><p className="m-0 text-[13px] text-ink-3">Nothing is open.</p></Centered>
  return <OpenFile key={id} id={id} editing={sub === 'edit'} nav={nav} />
}

function useLoad(id: string) {
  const source = useDriveSource()
  const [phase, setPhase] = useState<Phase>({ name: 'loading' })
  const [tries, setTries] = useState(0)
  useEffect(() => {
    let live = true
    setPhase({ name: 'loading' })
    ;(async () => {
      try {
        const { item } = await source.details(id)
        const kind = viewerKind(item.name)
        const data: Loaded = TEXTUAL.has(kind) ? { kind: 'text', ...(await source.readText(id)) } : kind === 'other' ? { kind: 'none' } : { kind: 'bytes', ...(await source.readBytes(id)) }
        if (live) setPhase({ name: 'ready', item, data })
      } catch (e) {
        if (!live) return
        if (e instanceof DriveSourceError && e.code === 'quota') {
          const { item } = await source.details(id).catch(() => ({ item: null }))
          if (item) return setPhase({ name: 'too-large', item })
        }
        setPhase({ name: 'failed' })
      }
    })()
    return () => { live = false }
  }, [source, id, tries])
  return { phase, retry: () => setTries((n) => n + 1), setPhase }
}

/** A newer version arriving while the file is open is announced, never swapped in under the reader (DW-1519). */
function useNewer(source: DriveSource, id: string, openedVersion: string | null) {
  const [newer, setNewer] = useState<VersionEntry | null>(null)
  useEffect(() => source.subscribe((e) => {
    if (e.kind !== 'item' || e.item.id !== id) return
    void source.versions(id).then((v) => { const cur = v.find((x) => x.current); if (cur && cur.id !== openedVersion) setNewer(cur) })
  }), [source, id, openedVersion])
  return [newer, () => setNewer(null)] as const
}

function OpenFile({ id, editing, nav }: { id: string; editing: boolean; nav: SectionProps['nav'] }) {
  const source = useDriveSource()
  const { phase, retry, setPhase } = useLoad(id)
  const openedVersion = phase.name === 'ready' && phase.data.kind !== 'none' ? phase.data.versionId : null
  const [newer, dismissNewer] = useNewer(source, id, openedVersion)
  const [panel, setPanel] = useState(false)
  useCommand('toggle-details-panel', useCallback(() => setPanel((p) => !p), []))
  const back = () => nav.back()

  if (phase.name === 'loading') return <DecryptingView name="" onCancel={back} />
  if (phase.name === 'failed') return <Frame name="" onBack={back}><DecryptFailedView onRetry={retry} onReport={() => nav.go({ section: 'settings', sub: 'report' })} /></Frame>
  if (phase.name === 'too-large') {
    return <Frame name={phase.item.name} onBack={back}><TooLargeView name={phase.item.name} size={phase.item.size ?? 0} onFreeUp={() => nav.go({ section: 'settings', sub: 'storage' })} onDownloadTo={() => void source.open(id)} /></Frame>
  }
  const { item, data } = phase
  const showNewer = () => { dismissNewer(); retry() }
  const banner = newer && !editing ? <NewerBanner text={`A newer version arrived from ${newer.deviceName ?? 'another device'}, ${hhmm(newer.createdAt)}.`} onShow={showNewer} /> : null
  const aside = panel ? <DetailsAside id={id} onClose={() => setPanel(false)} /> : null
  const body = editing && data.kind === 'text'
    ? <Editor item={item} initial={data} nav={nav} />
    : <Viewer item={item} data={data} nav={nav} banner={banner} onSaved={(d) => setPhase({ name: 'ready', item, data: { kind: 'text', ...d } })} />
  return <div className="flex min-h-0 flex-1">{body}{aside}</div>
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

function Viewer({ item, data, nav, banner, onSaved }: { item: DriveItem; data: Loaded; nav: SectionProps['nav']; banner: ReactNode; onSaved: (d: { text: string; versionId: string }) => void }) {
  void onSaved
  const source = useDriveSource()
  const kind = viewerKind(item.name)
  const [wrap, setWrap] = useState(false)
  const [mdMode, setMdMode] = useState<'rendered' | 'source'>('rendered')
  const [zoom, setZoom] = useState(1)
  const [find, setFind] = useState<string | null>(null)
  const bytes = data.kind === 'bytes' ? data.bytes : null
  const url = useBlobUrl(bytes, item.mime ?? 'application/octet-stream')
  const text = data.kind === 'text' ? data.text : ''
  const big = (item.size ?? 0) > MAX_EDITABLE_BYTES
  const openElsewhere = () => void source.open(item.id)
  const edit = () => nav.go({ section: 'open', id: item.id, sub: big ? null : 'edit' })
  const copy = () => void navigator.clipboard?.writeText(text)
  const openFile = (name: string) => void source.search(name).then((r) => { const hit = r.find((i) => i.name === name); if (hit) nav.go({ section: 'open', id: hit.id }) })
  const textual = TEXTUAL.has(kind)
  const editable = (kind === 'code' || kind === 'text' || kind === 'markdown') && !big
  const csvCols = kind === 'csv' ? parseTable(text) : null

  const meta = kind === 'code' || kind === 'text' ? `${languageLabel(item.name)} · ${lineCount(text)} lines · UTF-8` : kind === 'raw' ? undefined : undefined
  const actions = (
    <>
      {kind === 'markdown' && (
        <span role="group" aria-label="View" className="flex rounded-md border border-line p-0.5 text-[12px]">
          {(['rendered', 'source'] as const).map((m) => <button key={m} type="button" aria-pressed={mdMode === m} onClick={() => setMdMode(m)} className={`h-7 rounded px-3 capitalize ${mdMode === m ? 'bg-paper-3 text-ink' : 'text-ink-3'}`}>{m === 'rendered' ? 'Rendered' : 'Source'}</button>)}
        </span>
      )}
      {(kind === 'code' || kind === 'text') && <><ToolbarButton label="Wrap" pressed={wrap} onClick={() => setWrap((w) => !w)} /><ToolbarButton label="Copy" onClick={copy} /></>}
      {kind === 'pdf' && <><PdfZoom zoom={zoom} onZoom={setZoom} /><ToolbarButton label="Find" onClick={() => setFind('')} /></>}
      {kind === 'archive' && <ToolbarButton label="Open in another app…" onClick={openElsewhere} />}
      {(editable || kind === 'csv') && <ToolbarButton label="Edit" shortcut="Ctrl+E" onClick={kind === 'csv' ? () => nav.go({ section: 'open', id: item.id, sub: 'office' }) : edit} />}
      <MoreButton />
    </>
  )

  let body: ReactNode
  if (kind === 'raw' || kind === 'image') body = url ? (kind === 'image' ? <ImageViewer src={url} name={item.name} /> : <ImageViewer src={url} name={item.name} />) : null
  else if (kind === 'audio') body = <AudioView item={item} bytes={bytes} />
  else if (kind === 'pdf') body = url ? <div className="min-h-0 flex-1">{find !== null && <FindBar query={find} onQuery={setFind} onDone={() => setFind(null)} message="Find works in the page view" />}<div className="h-full"><PdfPreview blob={new Blob([bytes as BlobPart], { type: 'application/pdf' })} /></div></div> : null
  else if (kind === 'markdown') body = <MarkdownViewer text={text} mode={mdMode} onOpenFile={openFile} />
  else if (kind === 'csv') body = <CsvViewer text={text} />
  else if (kind === 'archive') body = <ZipView bytes={bytes} />
  else if (textual) body = <>{big && <OpenInAppNote onOpen={openElsewhere}>Read-only: this file is larger than 2 MB.</OpenInAppNote>}<CodeViewer text={text} wrap={wrap} limit={big ? READ_ONLY_LINES : undefined} />{big && <div className="flex-none border-t border-line px-4 py-1.5 font-mono text-[12px] text-ink-3">Showing the first {Math.min(READ_ONLY_LINES, lineCount(text))} of {lineCount(text).toLocaleString('en-GB')} lines</div>}</>
  else body = <Centered><p className="m-0 text-[13px] text-ink-2">{item.name} opens in another app.</p><ToolbarButton label="Open in another app…" onClick={openElsewhere} /></Centered>
  void csvCols

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <OpenToolbar name={item.name} meta={meta} onBack={() => nav.back()}>{actions}</OpenToolbar>
      {banner}
      {body}
    </div>
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

function AudioView({ item, bytes }: { item: DriveItem; bytes: Uint8Array | null }) {
  const [, tick] = useState(0)
  useEffect(() => player.onTime(() => tick((n) => n + 1)), [])
  useEffect(() => { if (bytes && player.playerState().current?.id !== item.id) player.loadTrack(item.id, bytes, { title: item.name.replace(/\.[^.]+$/, ''), artist: '' }, item.mime) }, [bytes, item])
  useCommand('save', () => {})
  const st = player.playerState()
  const length = Math.round(player.duration()) || 1
  const tracks: Track[] = [{ id: item.id, title: item.name.replace(/\.[^.]+$/, ''), artist: '', length }]
  return <AudioPlayer tracks={tracks} index={0} playing={st.playing} elapsed={Math.round(st.elapsed)} onToggle={player.toggle} onSeek={player.seek} fileName={item.name} />
}

// ---- The editor: Ctrl+S saves a new encrypted version; leaving with changes asks first ----
function Editor({ item, initial, nav }: { item: DriveItem; initial: { text: string; versionId: string }; nav: SectionProps['nav'] }) {
  const source = useDriveSource()
  const doc = useRef(initial.text)
  const base = useRef(initial)
  const [dirty, setDirty] = useState(false)
  const [version, setVersion] = useState(Number(initial.versionId.replace(/\D/g, '')) || 1)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [leave, setLeave] = useState(false)
  const [conflict, setConflict] = useState<{ theirs: { text: string; versionId: string }; entry: VersionEntry | undefined } | null>(null)
  const busy = useRef(false)

  const finish = (d: { text: string; versionId: string }) => {
    base.current = d
    setVersion(Number(d.versionId.replace(/\D/g, '')) || version + 1)
    setDirty(doc.current !== d.text)
    setSaved(true)
  }

  const save = useCallback(async (): Promise<boolean> => {
    if (busy.current) return false
    busy.current = true
    setSaving(true)
    try {
      finish(await source.saveText(item.id, doc.current, base.current.versionId))
      return true
    } catch (e) {
      if (e instanceof DriveSourceError && e.code === 'conflict') {
        const [theirs, versions] = await Promise.all([source.readText(item.id), source.versions(item.id)])
        setConflict({ theirs, entry: versions.find((v) => v.current) })
      }
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

  const goBack = () => (dirty ? setLeave(true) : nav.go({ section: 'open', id: item.id }))
  const resolve = async (choice: 'mine' | 'theirs' | 'both') => {
    if (!conflict) return
    if (choice === 'mine') {
      base.current = conflict.theirs
      setConflict(null)
      await save()
    } else if (choice === 'theirs') {
      setConflict(null)
      nav.go({ section: 'open', id: item.id })
    } else {
      const name = insertBeforeExtension(item.name, ' (This PC)')
      await source.upload(item.parentId, { kind: 'files', files: [new File([doc.current], name, { type: 'text/plain' })] })
      setConflict(null)
      nav.go({ section: 'open', id: item.id })
    }
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <OpenToolbar name={item.name} dirty={dirty} meta={editMeta(dirty, version, saving)} onBack={goBack}>
        <SaveButton onClick={() => void save()} disabled={!dirty || saving} />
        <MoreButton />
      </OpenToolbar>
      {saved && !dirty && <SavedMessage version={version} onVersions={() => nav.go({ section: 'files', sub: 'versions', id: item.id })} />}
      <TextEditor name={item.name} initialDoc={initial.text} onChange={(d) => { doc.current = d; setDirty(d !== base.current.text); setSaved(false) }} onSave={() => void save()} />
      {leave && <UnsavedAlert name={item.name} onKeep={() => setLeave(false)} onDiscard={() => nav.go({ section: 'open', id: item.id })} onSave={() => void save().then((ok) => { if (ok) nav.go({ section: 'open', id: item.id }); else setLeave(false) })} />}
      {conflict && (
        <EditConflictDialog
          name={item.name}
          changedOn={`Changed on ${conflict.entry?.deviceName ?? 'another device'} while you were editing`}
          mine={{ label: 'Your edit', when: 'this PC', note: `not saved · ${lineDeltaText(doc.current, conflict.theirs.text)}`, preview: doc.current }}
          theirs={{ label: conflict.entry?.deviceName ?? 'Another device', when: conflict.entry ? hhmm(conflict.entry.createdAt) : '', note: `version ${Number(conflict.theirs.versionId.replace(/\D/g, '')) || ''}`, preview: conflict.theirs.text }}
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
  const [type, setType] = useState<FileType['id']>('md')
  const [step, setStep] = useState<'type' | 'name'>('type')
  const [name, setName] = useState('Untitled')
  const [ext, setExt] = useState('.md')
  const [taken, setTaken] = useState<string[]>([])
  useEffect(() => { void source.list(parentId).then((l) => setTaken(l.items.map((i) => i.name.toLowerCase()))) }, [source, parentId])
  const cancel = () => nav.back()
  const full = `${name.trim()}${ext}`
  const error = step === 'name' && name !== 'Untitled' ? nameError(name, taken.includes(full.toLowerCase())) : null
  const create = async () => {
    const err = nameError(name, taken.includes(full.toLowerCase()))
    if (err) return
    await source.upload(parentId, { kind: 'files', files: [new File([''], full, { type: 'text/plain' })] })
    const hit = (await source.list(parentId)).items.find((i) => i.name === full)
    nav.go({ section: 'open', id: hit?.id ?? null, sub: hit ? 'edit' : null })
  }
  const pick = () => {
    const t = FILE_TYPES.find((x) => x.id === type)!
    if (t.office) return nav.go({ section: 'open', id: parentId, sub: 'new-document' })
    setExt(t.ext)
    setStep('name')
  }
  return step === 'type'
    ? <NewFileTypeDialog selected={type} onSelect={setType} onContinue={pick} onCancel={cancel} />
    : <NewFileNameDialog markdown={type === 'md'} name={name} ext={ext} error={error} onName={setName} onExt={setExt} onCreate={() => void create()} onCancel={cancel} />
}
