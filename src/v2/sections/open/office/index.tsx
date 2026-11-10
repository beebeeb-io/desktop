// Office on this PC (DW-1507..1562): Docs, Sheets and Slides in the Drive window. The document is read with
// DriveSource.readBytes (decrypted in Rust), edited by the engine running in this window, and saved with
// DriveSource.saveBytes under the version it was opened at: a newer version is a conflict, never an overwrite.
// The engine itself comes from Rust's cache (office_v2), downloaded once and hash-checked there.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  applyTemplate, CALC_PALETTE_ENTRIES, DriveSourceError, IMPRESS_LAYOUTS, layoutDispatchArgs, resolveOfficeFileKind, useDriveSource, WRITER_PALETTE_ENTRIES,
  type DriveItem, type OfficeApp, type OfficeBridge, type OutlineHeading, type TemplateDef, type UnoArg,
} from '@beebeeb/shared/drive-ui'
import { useCommand } from '../../../shortcuts'
import type { Navigation } from '../../../routes'
import { SavedMessage, UnsavedAlert } from '../Editing'
import { ShareHost, VersionsHost } from '../hosts'
import { openUrl } from '../../../../desktopApi'
import { useDriveExtras } from '../../../source/extras'
import { EngineFrame, FormulaBar, OutlinePane } from './Canvas'
import { engineHostUrl, engineStatus, failureDetail, prepareEngine, waitForBridge, type EngineProgress } from './engine'
import type { RelayBridge } from './relay'
import { AboutPanel, CommandPalette, ConflictDialog, defaultNewName, FailedView, isLegacy, LegacyNote, NewDocumentDialog, OFFICE_SOURCE_URL, OfficeStatus, PreparingView, TableSizeDialog, type ConflictChoice, type PaletteRow } from './Parts'
import { OFFICE_TABS, type Control } from './Ribbon'
import { APP_LABEL, OfficeShell } from './View'
import { createKeptCopy, tagContent } from '../model'
import { useVersionWatch } from '../versionWatch'
import { officeSave } from './save'
import { useOwnFind } from '../find'
import { failedWords } from '../../photos/Dialogs'
import { showToast } from '../../../chrome/Toast'

/** A new document's file is written, and the engine has not added it yet: the save waits for that (#273 finding 11). */
export const NOT_INDEXED = 'Not saved yet. Beebeeb is still adding the new document to the folder; try again in a moment. Your edit is still here.'

/** Why a save did not happen, in words: a taken name says so, never "try again" alone (#273 finding 2). */
export function saveFailure(e: unknown): string {
  if (e instanceof DriveSourceError && e.code === 'unsupported') return 'This PC can’t save a new file from the editor yet.'
  if (e instanceof DriveSourceError && e.code === 'name-taken') return failedWords(e, 'save it. Your edit is still here')
  return 'Not saved. Your edit is still here; try again.'
}

type Engine = { name: 'checking' } | { name: 'preparing'; progress: EngineProgress } | { name: 'booting'; src: string } | { name: 'ready'; src: string } | { name: 'failed'; detail: string }

const extOf = (n: string) => (n.lastIndexOf('.') > 0 ? n.slice(n.lastIndexOf('.') + 1).toLowerCase() : '')
const ctrl = (s?: string) => s?.replace(/⌘/g, 'Ctrl+').replace(/⇧/g, 'Shift+').replace(/⌥/g, 'Alt+')
const hhmm = (d: Date) => d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })
const numberFmt = new Intl.NumberFormat('en-GB')

/** Is this file one the editor opens? */
export const isOfficeFile = (item: Pick<DriveItem, 'name' | 'mime'>) => resolveOfficeFileKind(item.mime, item.name) !== null

/** Prepare the engine through Rust, then hand back the frame's src. */
function useEngine() {
  const [engine, setEngine] = useState<Engine>({ name: 'checking' })
  const [tries, setTries] = useState(0)
  useEffect(() => {
    let live = true
    ;(async () => {
      try {
        let status = await engineStatus()
        if (status.state === 'unpublished') throw new Error('unpublished: no bundle')
        if (status.state === 'missing') {
          if (live) setEngine({ name: 'preparing', progress: { loaded: 0, total: status.totalBytes } })
          status = await prepareEngine((progress) => { if (live) setEngine({ name: 'preparing', progress }) })
        }
        if (live && status.version) setEngine({ name: 'booting', src: engineHostUrl(status.version) })
      } catch (e) {
        if (live) setEngine({ name: 'failed', detail: failureDetail(e instanceof Error ? e.message : String(e)) })
      }
    })()
    return () => { live = false }
  }, [tries])
  return { engine, setEngine, retry: () => setTries((n) => n + 1) }
}

export type OfficeTarget = { kind: 'file'; id: string } | { kind: 'new'; parentId: string | null; app: OfficeApp; template: TemplateDef['id']; name: string }

export function OfficeEditor({ target, nav }: { target: OfficeTarget; nav: Navigation }) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [share, setShare] = useState(false)
  const [versionsOpen, setVersionsOpen] = useState(false)
  const { engine, setEngine, retry } = useEngine()
  const frameRef = useRef<HTMLIFrameElement>(null)
  const bridgeRef = useRef<RelayBridge | null>(null)
  // The relay listens on this window: stop it when the editor goes.
  useEffect(() => () => { bridgeRef.current?.close(); bridgeRef.current = null }, [])
  const [item, setItem] = useState<DriveItem | null>(null)
  const [path, setPath] = useState<string[]>(['Files'])
  // `versionId` is Rust's tag of the bytes on this PC; null for a new document Rust has written that the engine has not
  // indexed yet (its provisional id reads only once indexed). The tag then comes with the engine's next event.
  const [doc, setDoc] = useState<{ bytes: Uint8Array; versionId: string | null } | null>(null)
  const [versionNumber, setVersionNumber] = useState(0)
  const [savedAt, setSavedAt] = useState<Date | null>(null)
  const [dirty, setDirty] = useState(target.kind === 'new')
  const [saving, setSaving] = useState(false)
  const [justSaved, setJustSaved] = useState(false)
  const [conflict, setConflict] = useState<{ latest: number; latestId: string } | null>(null)
  const [tab, setTab] = useState('Home')
  const [palette, setPalette] = useState(false)
  const [about, setAbout] = useState(false)
  const [tableDialog, setTableDialog] = useState(false)
  const [leave, setLeave] = useState(false)
  const [outline, setOutline] = useState<OutlineHeading[]>([])
  const [stats, setStats] = useState<{ words: number | null; language: string | null }>({ words: null, language: null })
  const [recent, setRecent] = useState<DriveItem[]>([])
  const [zoom, setZoom] = useState(100)
  const [layout, setLayout] = useState(IMPRESS_LAYOUTS[1].id)
  const [error, setError] = useState<string | null>(null)
  const name = target.kind === 'new' ? target.name : item?.name ?? ''
  const app: OfficeApp = target.kind === 'new' ? target.app : resolveOfficeFileKind(item?.mime, name)?.app ?? 'writer'
  const fileId = target.kind === 'file' ? target.id : item?.id ?? null

  /** Rust's tag of the document on this PC; null while the engine has not indexed a file this window just made. */
  const tagOf = useCallback(async (id: string): Promise<string | null> => {
    try { return (await source.readBytes(id)).versionId } catch (e) {
      if (e instanceof DriveSourceError && e.code === 'not-found') return null
      throw e
    }
  }, [source])
  // The save's state is the engine's: "Saved on this PC · encrypting…" until it has uploaded the save, then "Saved ·
  // encrypted · version N" with Rust's number (#273 finding 12, as the text editor).
  const { note, savedHere } = useVersionWatch(source, fileId, doc ? versionNumber : null, {
    readTag: (id) => source.readBytes(id).then((r) => r.versionId),
    saved: (version, tag) => { setVersionNumber(version); setDoc((d) => (d && d.versionId !== null && tagContent(d.versionId) === tagContent(tag) ? { ...d, versionId: tag } : d)) },
  })
  // A new document's tag, once the engine has indexed the file Rust wrote: on its event, never a timer guess (finding 11).
  useEffect(() => {
    if (!fileId || !doc || doc.versionId !== null) return
    return source.subscribe((e) => {
      if (e.kind !== 'changed' && e.kind !== 'item') return
      void tagOf(fileId).then((tag) => { if (tag) setDoc((d) => (d && d.versionId === null ? { ...d, versionId: tag } : d)) }, () => {})
    })
  }, [source, fileId, doc, tagOf])

  // The document: details, the decrypted bytes and the version it was opened at.
  useEffect(() => {
    if (target.kind !== 'file') return
    let live = true
    ;(async () => {
      try {
        const [{ item: it, path: p, versionCount }, bytes] = await Promise.all([source.details(target.id), source.readBytes(target.id)])
        if (!live) return
        setItem(it)
        // The path starts at the root crumb (Files), as drive_details gives it.
        setPath(p.length ? p.map((c) => c.name) : ['Files'])
        setDoc(bytes)
        // Rust's number: the server's version (drive_details' versionCount), never one counted in the window.
        setVersionNumber(versionCount)
        setSavedAt(new Date(it.modifiedAt))
      } catch {
        if (live) setEngine({ name: 'failed', detail: 'The document could not be decrypted on this PC.' })
      }
    })()
    void source.recent(5).then((r) => { if (live) setRecent(r) }).catch(() => {})
    return () => { live = false }
  }, [source, target, setEngine])

  const refreshDoc = useCallback(async (b: OfficeBridge) => {
    const [o, s] = await Promise.all([b.getOutline().catch(() => []), b.getDocStats().catch(() => null)])
    setOutline(o)
    if (s) setStats({ words: s.words, language: s.language })
  }, [])

  // Boot: once the frame loads, wait for the engine, then open (or create) the document in it.
  const onFrameLoad = useCallback(() => {
    const frame = frameRef.current
    if (!frame) return
    void (async () => {
      try {
        bridgeRef.current?.close()
        bridgeRef.current = null
        const b = await waitForBridge(frame)
        bridgeRef.current = b
        if (target.kind === 'new') {
          await b.newDocument(target.app)
          if (target.app === 'writer') await applyTemplate(b, target.template)
        } else if (doc) {
          await b.open(doc.bytes, name)
        }
        await b.onModifiedChange((m) => setDirty(m))
        await b.onSelectionChange(() => { void refreshDoc(b) })
        await refreshDoc(b)
        setEngine((e) => ('src' in e ? { name: 'ready', src: e.src } : e)) // keep the frame: it is the document
      } catch (e) {
        setEngine({ name: 'failed', detail: failureDetail(e instanceof Error ? e.message : 'boot') })
      }
    })()
  }, [doc, name, target, refreshDoc, setEngine])

  // A new document's first save and "Keep both" make a new file: Rust writes it into the folder (drive_create_file).
  const upload = useCallback(async (fileName: string, bytes: Uint8Array) => {
    const parentId = target.kind === 'new' ? target.parentId : item?.parentId ?? null
    return extras.createFile(parentId, fileName, bytes)
  }, [extras, target, item])

  const save = useCallback(async (base?: string): Promise<boolean> => {
    const b = bridgeRef.current
    if (!b || saving) return false
    setSaving(true)
    setError(null)
    try {
      const bytes = await b.save()
      // The first save makes the file: once Rust has written it, that save is done, whether or not the engine has indexed
      // it yet; its tag comes now or with the engine's event (#273 finding 11). A later save names the tag it was read at.
      const r = await officeSave({ creating: target.kind === 'new' && !item, fileId, doc, base }, bytes, {
        make: (x) => upload(name, x), tagOf, saveBytes: (id, x, tag) => source.saveBytes(id, x, tag),
      })
      if (r.kind === 'not-indexed') { setError(NOT_INDEXED); return false }
      if (r.kind === 'made') setItem(r.item)
      setDoc({ bytes, versionId: r.versionId })
      savedHere(r.kind === 'made' ? null : r.versionId)
      setSavedAt(new Date())
      setDirty(false)
      setJustSaved(true)
      setConflict(null)
      return true
    } catch (e) {
      if (e instanceof DriveSourceError && e.code === 'conflict' && fileId) {
        const versions = await source.versions(fileId).catch(() => [])
        const latest = versions.find((v) => v.current) ?? versions[0]
        setConflict({ latest: versions.length, latestId: latest?.id ?? '' })
      } else {
        setError(saveFailure(e))
      }
      return false
    } finally {
      setSaving(false)
    }
  }, [saving, target, item, upload, name, source, fileId, doc, tagOf, savedHere])

  const onConflict = async (c: ConflictChoice) => {
    if (!conflict) return
    if (c === 'save-as-new-version') { await save(conflict.latestId); return }
    if (c === 'discard') {
      const b = bridgeRef.current
      if (!fileId || !b) return
      const fresh = await source.readBytes(fileId)
      await b.open(fresh.bytes, name)
      setDoc(fresh); setVersionNumber((await source.details(fileId).catch(() => null))?.versionCount ?? conflict.latest); setConflict(null); setDirty(false)
      return
    }
    const b = bridgeRef.current
    if (!b) return
    setSaving(true)
    try {
      // Mine goes next to it under a free name (the text editor's rule); this file stays theirs (#273 finding 2).
      const bytes = await b.save()
      const parentId = target.kind === 'new' ? target.parentId : item?.parentId ?? null
      const kept = await createKeptCopy(name, 'edited on this PC', () => source.list(parentId).then((l) => l.items.map((i) => i.name)), (n) => upload(n, bytes))
      // This file stays theirs: the editor shows their version again, as the text editor does (self-review).
      if (fileId) {
        const fresh = await source.readBytes(fileId)
        await b.open(fresh.bytes, name)
        setDoc(fresh)
        setVersionNumber((await source.details(fileId).catch(() => null))?.versionCount ?? conflict.latest)
      }
      setConflict(null); setDirty(false)
      showToast({ kind: 'done', text: `Your edit is saved as ${kept.name}` })
    } catch (e) {
      setError(saveFailure(e))
    } finally { setSaving(false) }
  }

  const run = useCallback((command: string, args?: UnoArg[]) => { void bridgeRef.current?.dispatch(command, args) }, [])
  const onControl = (c: Control) => {
    if (c.select) {
      // A chevron choice carries the picked value; dispatch the engine command the bridge has for it.
      if (c.select.command) run(c.select.command, c.select.args?.(c.select.value))
      return
    }
    if (c.id === 'insert-table') return setTableDialog(true)
    if (c.id === 'present') return run('.uno:Presentation')
    if (c.id === 'insert-image' || c.id === 'insert-link') return setError('Use Ctrl+K › Insert for images and links.')
    if (c.uno) run(c.uno)
  }

  useCommand('save', useCallback(() => { if (dirty) void save() }, [dirty, save]))
  // Ctrl+F never leaves the editor for Search, whatever route opened it (#273 finding 14): the engine's frame takes the
  // focus, and with it the next Ctrl+F (the engine's own find).
  useOwnFind(true, useCallback(() => { frameRef.current?.focus() }, []))
  useCommand('commands', useCallback(() => setPalette(true), []))
  useCommand('present', useCallback(() => { if (app === 'impress') run('.uno:Presentation') }, [app, run]))

  const paletteRows = useMemo<(PaletteRow & { command?: string; args?: UnoArg[]; fileId?: string })[]>(() => {
    const cmds = app === 'calc' ? CALC_PALETTE_ENTRIES : WRITER_PALETTE_ENTRIES
    return [
      ...cmds.map((e) => ({ id: e.id, label: e.label, group: (e.command === '.uno:StyleApply' ? 'Styles' : e.group === 'Insert' || e.group === 'Document' ? e.group : 'Format') as PaletteRow['group'], shortcut: ctrl(e.shortcut), command: e.command, args: e.args })),
      ...recent.filter((r) => !r.isFolder && r.id !== fileId).map((r) => ({ id: `file-${r.id}`, label: r.name, group: 'Files' as const, fileId: r.id })),
    ]
  }, [app, recent, fileId])

  // Back: the folder the document is in; with unsaved changes it asks first (DW-1507).
  const folderId = target.kind === 'new' ? target.parentId : item?.parentId ?? null
  const toFolder = () => nav.go({ section: 'files', id: folderId })
  const back = () => (dirty ? setLeave(true) : toFolder())
  const otherApp = () => { if (fileId) void source.open(fileId) }
  const meta = item ? `${APP_LABEL[app]} · opens in the editor` : undefined
  const ext = extOf(name)
  const left = app === 'calc' ? ['Sheet 1'] : app === 'impress' ? ['English (UK)'] : [stats.words != null ? `${numberFmt.format(stats.words)} words` : null, stats.language ?? 'English (UK)'].filter((x): x is string => !!x)

  const body = engine.name === 'preparing' ? <PreparingView loaded={engine.progress.loaded} total={engine.progress.total} />
    : engine.name === 'failed' ? <FailedView detail={engine.detail} onRetry={retry} onOtherApp={otherApp} />
    : engine.name === 'checking' ? <PreparingView loaded={0} total={0} />
    : (
      <>
        {app === 'writer' && <OutlinePane items={outline.map((h) => [h.text, Math.max(0, h.level - 1)] as [string, number])} onGo={(i) => { void bridgeRef.current?.goToHeading(i) }} />}
        <div className="flex min-w-0 flex-1 flex-col">
          {app === 'calc' && <FormulaBar cell="A1" formula="" onGo={(ref) => { run('.uno:GoToCell', [{ name: 'ToPoint', value: ref }]); return true }} />}
          {(doc || target.kind === 'new') && 'src' in engine ? <EngineFrame src={engine.src} frameRef={frameRef} onLoad={onFrameLoad} /> : engine.name === 'ready' ? null : <PreparingView loaded={0} total={0} />}
        </div>
      </>
    )

  return (
    <>
      <OfficeShell
        app={app} path={path} name={name} meta={meta} dirty={dirty} saving={saving} tab={OFFICE_TABS[app].includes(tab) ? tab : 'Home'} onTab={setTab} onRun={onControl}
        onSave={() => void save()} onPalette={() => setPalette(true)} onShare={() => { if (item) setShare(true) }} onBack={back}
        dimmed={engine.name !== 'ready'} layout={{ current: layout, onPick: (id) => { setLayout(id); const l = IMPRESS_LAYOUTS.find((x) => x.id === id); if (l) run('.uno:AssignLayout', layoutDispatchArgs(l)) } }}
        note={<>
          {isLegacy(ext) && <LegacyNote ext={ext} onOtherApp={otherApp} />}
          {justSaved && !dirty && note && <SavedMessage note={note} onVersions={() => { if (item) setVersionsOpen(true) }} />}
          {error && <div role="alert" className="flex h-9 flex-none items-center border-b border-line bg-paper-2 px-4 text-[13px] text-ink">{error}</div>}
        </>}
        status={<OfficeStatus left={left} dirty={dirty} conflict={!!conflict} version={versionNumber} time={savedAt ? hhmm(savedAt) : ''} loading={engine.name !== 'ready' && engine.name !== 'failed'} failed={engine.name === 'failed'} uploading={note?.kind === 'local'} zoom={zoom} onZoom={(z) => { setZoom(z); void bridgeRef.current?.setZoom(z) }} onAbout={() => setAbout(true)} />}
      >
        {body}
      </OfficeShell>
      {palette && <CommandPalette rows={paletteRows} onClose={() => setPalette(false)} onRun={(r) => {
        setPalette(false)
        const row = paletteRows.find((x) => x.id === r.id)
        if (row?.fileId) nav.go({ section: 'open', id: row.fileId, sub: null })
        else if (row?.command) run(row.command, row.args)
      }} />}
      {conflict && <ConflictDialog latest={conflict.latest} opened={versionNumber} saving={saving} onChoose={(c) => void onConflict(c)} onCancel={() => setConflict(null)} />}
      {about && <AboutPanel onClose={() => setAbout(false)} noticesAvailable={false} onOpenLink={(w) => { if (w === 'source') void openUrl(OFFICE_SOURCE_URL) }} />}
      {tableDialog && <TableSizeDialog onCancel={() => setTableDialog(false)} onInsert={(rows, cols) => { setTableDialog(false); run('.uno:InsertTable', [{ name: 'Rows', value: rows }, { name: 'Columns', value: cols }]) }} />}
      {leave && <UnsavedAlert name={name} onKeep={() => setLeave(false)} onDiscard={toFolder} onSave={() => void save().then((ok) => { if (ok) toFolder(); else setLeave(false) })} />}
      {share && item && <ShareHost items={[item]} onClose={() => setShare(false)} />}
      {versionsOpen && item && <VersionsHost item={item} onClose={() => setVersionsOpen(false)} />}
    </>
  )
}

/** "New document" (DW-1558): the dialog, then the editor on the new file in the folder you're in. */
export function NewDocumentFlow({ parentId, nav }: { parentId: string | null; nav: Navigation }) {
  const [kind, setKind] = useState<OfficeApp>('writer')
  const [template, setTemplate] = useState<TemplateDef['id']>('blank')
  const [name, setName] = useState(defaultNewName('writer'))
  const [made, setMade] = useState<OfficeTarget | null>(null)
  if (made) return <OfficeEditor target={made} nav={nav} />
  return (
    <NewDocumentDialog kind={kind} template={template} name={name} onKind={(k) => { setKind(k); setTemplate('blank') }} onTemplate={setTemplate} onName={setName}
      onCancel={() => nav.go({ section: 'files', id: parentId })} onCreate={() => setMade({ kind: 'new', parentId, app: kind, template, name: name.trim() })} />
  )
}
