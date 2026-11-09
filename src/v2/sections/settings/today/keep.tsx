// Keep on this PC (DW-0784 family): choose which folders stay on this PC, many at once, and see what each choice
// does before it happens. Checked = kept (everything inside, also what is added later); nothing changes until Apply.
import { useEffect, useState, useSyncExternalStore } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { command, commandUnavailableLabel, formatBytes, type CommandResult, type VaultItem } from '../../../../desktopApi'
import { Btn, Dialog, mono } from '../shell/kit'
import { useSettings } from '../shell/context'
import { Check, ErrorBanner, Skeleton } from './ui'

/** The dialog's tree row. `bytes` and `files` are the reported subtree totals, or null when the read left them out. */
export interface KeepNode { id: string; name: string; gb: number; bytes: number | null; files: number | null; children?: KeepNode[] }

const reported = (value: number | undefined): number | null => (typeof value === 'number' && Number.isFinite(value) ? value : null)

/**
 * The dialog's tree from the app's own reads: `list_vault_folders` (id, name, subtree size, children) and
 * `get_selective_sync` (the ids of folders that are online only). A folder is kept when it is not excluded and no
 * ancestor is; a node is added to `checked` only when its whole subtree stays, so a folder holding an excluded
 * descendant is "some folders kept" and its kept children are listed one by one. `size_bytes` is decimal GB here.
 * A folder the vault marks `excluded` is not kept either, even when the id list omitted it.
 */
export function keepTreeFrom(folders: VaultItem[], excluded: string[]): { tree: KeepNode[]; checked: Set<string> } {
  const ex = new Set(excluded)
  const mark = (nodes: VaultItem[]) => {
    for (const node of nodes) {
      if (node.excluded) ex.add(node.id)
      if (node.children?.length) mark(node.children)
    }
  }
  mark(folders)
  const toNode = (f: VaultItem): KeepNode => {
    const kids = (f.children ?? []).map(toNode)
    const bytes = reported(f.size_bytes)
    return { id: f.id, name: f.name, gb: bytes == null ? 0 : bytes / 1e9, bytes, files: reported(f.file_count), ...(kids.length ? { children: kids } : {}) }
  }
  const tree = folders.map(toNode)
  const hasExclusion = (n: KeepNode): boolean => ex.has(n.id) || (n.children ?? []).some(hasExclusion)
  const checked = new Set<string>()
  const walk = (n: KeepNode, underExcluded: boolean) => {
    if (underExcluded || ex.has(n.id)) return
    if (!hasExclusion(n)) { checked.add(n.id); return }
    ;(n.children ?? []).forEach((c) => walk(c, false))
  }
  tree.forEach((n) => walk(n, false))
  return { tree, checked }
}

interface KeepStore { checked: Set<string>; initial: Set<string> }
/** The tree the last successful read answered; the dialog's helpers work on it, never on a board tree. */
let tree: KeepNode[] = []
let store: KeepStore = { checked: new Set(), initial: new Set() }
/** False after a failed read, so reset cannot check every folder the vault did not confirm. */
let readOk = false
const subs = new Set<() => void>()
const put = (s: KeepStore) => { store = s; subs.forEach((f) => f()) }
const useKeep = () => useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f) } }, () => store)
/** Back to the last successful read. A failed read leaves the selection alone: it must not check every folder. */
export const resetKeep = () => { if (!readOk) return; put({ checked: new Set(store.initial), initial: new Set(store.initial) }) }
/** Frame helper only (DW-0784's "changed" state): the board's preset tree has no live counterpart, so this never
 *  touches the app's data. The board's drawn state returns in M4's board-data pass. */
export const presetKeep = () => undefined

const ids = (n: KeepNode): string[] => [n.id, ...(n.children ?? []).flatMap(ids)]
const contains = (n: KeepNode, id: string): boolean => ids(n).includes(id)
const leaves = (nodes: KeepNode[]): KeepNode[] => nodes.flatMap((n) => (n.children ? leaves(n.children) : [n]))
const find = (nodes: KeepNode[], id: string): KeepNode | undefined => { for (const n of nodes) { if (n.id === id) return n; const c = n.children && find(n.children, id); if (c) return c } }

/** Effective state of a node under a set of checked folders. */
function stateOf(n: KeepNode, set: Set<string>, inherited = false): 'on' | 'mixed' | 'off' {
  if (inherited || set.has(n.id)) return 'on'
  const kids = (n.children ?? []).map((c) => stateOf(c, set))
  return kids.some((k) => k !== 'off') ? 'mixed' : 'off'
}
const leafOn = (leaf: KeepNode, set: Set<string>, nodes = tree, inherited = false): boolean => {
  for (const n of nodes) {
    const on = inherited || set.has(n.id)
    if (n.id === leaf.id) return on
    if (n.children && contains(n, leaf.id)) return leafOn(leaf, set, n.children, on)
  }
  return false
}

function uncheck(nodes: KeepNode[], target: string, set: Set<string>, on: boolean) {
  for (const n of nodes) {
    const has = on || set.has(n.id)
    if (n.id === target) { ids(n).forEach((i) => set.delete(i)); return }
    if (n.children && contains(n, target)) {
      if (has) { set.delete(n.id); n.children.forEach((c) => { if (!contains(c, target)) set.add(c.id) }) }
      uncheck(n.children, target, set, has)
      return
    }
  }
}
function toggle(n: KeepNode) {
  const set = new Set(store.checked)
  if (stateOf(n, set, ancestorOn(n.id, set)) === 'on') uncheck(tree, n.id, set, false)
  else { ids(n).forEach((i) => set.delete(i)); set.add(n.id) }
  put({ ...store, checked: set })
}
const ancestorOn = (id: string, set: Set<string>, nodes = tree, inherited = false): boolean => {
  for (const n of nodes) {
    const on = inherited || set.has(n.id)
    if (n.id === id) return inherited
    if (n.children && contains(n, id)) return ancestorOn(id, set, n.children, on)
  }
  return false
}

const sizeText = (n: KeepNode): string | null => (n.bytes == null ? null : formatBytes(n.bytes))
/** The kept roots: a checked folder already includes its subtree, so its children are not added again. */
function chosen(nodes: KeepNode[], set: Set<string>): KeepNode[] {
  const out: KeepNode[] = []
  for (const node of nodes) {
    if (set.has(node.id)) out.push(node)
    else if (node.children?.length) out.push(...chosen(node.children, set))
  }
  return out
}
function reportedSum(nodes: KeepNode[], key: 'bytes' | 'files'): number | null {
  let seen = false
  let total = 0
  for (const node of nodes) {
    const value = node[key]
    if (value == null) continue
    seen = true
    total += value
  }
  return seen ? total : null
}
export function plan(s: KeepStore) {
  const ls = leaves(tree)
  const was = (l: KeepNode) => leafOn(l, s.initial)
  const now = (l: KeepNode) => leafOn(l, s.checked)
  const leaving = ls.filter((l) => was(l) && !now(l))
  const coming = ls.filter((l) => !was(l) && now(l))
  const on = ls.filter(now).reduce((t, l) => t + l.gb, 0)
  const all = ls.reduce((t, l) => t + l.gb, 0)
  const bytes = reportedSum(chosen(tree, s.checked), 'bytes')
  const totalBytes = reportedSum(tree, 'bytes')
  return {
    leaving, coming, leaveGb: leaving.reduce((t, l) => t + l.gb, 0), comeGb: coming.reduce((t, l) => t + l.gb, 0),
    on, off: all - on, changed: leaving.length + coming.length > 0,
    files: reportedSum(chosen(tree, s.checked), 'files'), bytes,
    offBytes: bytes == null || totalBytes == null ? null : Math.max(0, totalBytes - bytes),
  }
}

function Tree({ nodes, depth, openSet, flip }: { nodes: KeepNode[]; depth: number; openSet: Set<string>; flip: (id: string) => void }) {
  const s = useKeep()
  return (
    <ul role={depth === 0 ? 'tree' : 'group'} aria-label={depth === 0 ? 'Folders' : undefined}>
      {nodes.map((n) => {
        const st = stateOf(n, s.checked, ancestorOn(n.id, s.checked))
        const was = stateOf(n, s.initial, ancestorOn(n.id, s.initial))
        const leaving = was === 'on' && st === 'off'
        const coming = was !== 'on' && st === 'on'
        const open = openSet.has(n.id)
        return (
          <li key={n.id} role="treeitem" aria-selected={false} aria-expanded={n.children ? open : undefined} aria-label={`${n.name}, ${st === 'mixed' ? 'mixed' : st === 'on' ? 'kept' : 'online only'}`}>
            <div className="flex h-9 items-center gap-2 pr-3" style={{ paddingLeft: 8 + depth * 20 }} data-testid="keep-row">
              {n.children ? (
                <button type="button" aria-label={`${open ? 'Close' : 'Open'} ${n.name}`} onClick={() => flip(n.id)} className="grid h-5 w-5 place-items-center text-ink-3"><Icon name="chevron-right" size={12} className={open ? 'rotate-90' : ''} /></button>
              ) : <span className="w-5" />}
              <Check state={st} label={n.name} onChange={() => toggle(n)} />
              <Icon name="folder" size={14} className="text-ink-3" />
              <span className="flex-1 truncate text-[13px] text-ink">{n.name}</span>
              <span className={`text-[12px] ${coming || leaving ? 'text-ink' : 'text-ink-3'} ${mono}`}>
                {sizeText(n) == null ? '' : leaving ? `Leaves this PC · ${sizeText(n)}` : coming ? `Downloads · ${sizeText(n)}` : sizeText(n)}
              </span>
              <span className="w-[112px] text-right text-[12px] text-ink-3">{st === 'on' ? 'Kept' : st === 'mixed' ? 'Some folders kept' : 'Online only'}</span>
            </div>
            {n.children && open ? <Tree nodes={n.children} depth={depth + 1} openSet={openSet} flip={flip} /> : null}
          </li>
        )
      })}
    </ul>
  )
}

/** The folders that are not kept, at the highest folder the person turned off, not each file-less leaf under it. */
function excludedIds(nodes: KeepNode[], set: Set<string>): string[] {
  const out: string[] = []
  for (const node of nodes) {
    if (set.has(node.id)) continue
    const kids = node.children ?? []
    if (kids.some((child) => stateOf(child, set) !== 'off')) out.push(...excludedIds(kids, set))
    else out.push(node.id)
  }
  return out
}
function apply(s: KeepStore): Promise<CommandResult<void>> {
  return command<void>('set_selective_sync', { excluded: excludedIds(tree, s.checked) })
}
function wrote(result: CommandResult<void>, setError: (message: string | null) => void, close: () => void) {
  if (!result.ok) { setError(result.unsupported ? commandUnavailableLabel('set_selective_sync') : result.reason); return }
  setError(null)
  close()
}

/** The app's own answers, mapped onto the dialog's tree; null when either read fails. */
async function readTree(): Promise<{ tree: KeepNode[]; checked: Set<string> } | null> {
  const [folders, excluded] = await Promise.all([command<VaultItem[]>('list_vault_folders'), command<string[]>('get_selective_sync')])
  if (!folders.ok || !excluded.ok) { readOk = false; return null }
  readOk = true
  return keepTreeFrom(folders.value, excluded.value)
}

type Phase = 'loading' | 'ready' | 'error' | 'empty'
export function KeepDialog({ arg }: { arg?: string }) {
  const { close, open } = useSettings()
  const s = useKeep()
  const [openSet, setOpenSet] = useState(new Set<string>())
  const [phase, setPhase] = useState<Phase>(arg === 'error' ? 'error' : arg === 'empty' ? 'empty' : 'loading')
  const [attempt, setAttempt] = useState(0)
  const [writeError, setWriteError] = useState<string | null>(null)
  useEffect(() => {
    if (arg === 'error' || arg === 'empty') return
    let live = true
    void readTree().then((read) => {
      if (!live) return
      if (!read) { setPhase('error'); return }
      tree = read.tree
      put({ checked: new Set(read.checked), initial: new Set(read.checked) })
      setPhase(read.tree.length === 0 ? 'empty' : 'ready')
    })
    return () => { live = false }
  }, [arg, attempt])
  const p = plan(s)
  const go = () => { if (p.leaving.length) open('keep-confirm'); else void apply(s).then((result) => wrote(result, setWriteError, close)) }
  const keptLine = `${p.files == null ? '' : `${p.files} ${p.files === 1 ? 'file' : 'files'} · `}${p.bytes == null ? '' : `${formatBytes(p.bytes)} on this PC`}${p.offBytes == null ? '' : ` · ${formatBytes(p.offBytes)} online only`}`
  return (
    <Dialog labelId="keep-h" title="Keep on this PC" wide onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={go} disabled={!p.changed || phase !== 'ready'}>Apply</Btn></>}>
      <p>Folders you choose stay on this PC. Everything else downloads when you open it.</p>
      {writeError ? <p role="alert" className="mt-2 text-[12px] text-red-text">{writeError}</p> : null}
      {phase === 'error' ? (
        <div className="mt-3"><ErrorBanner title="Couldn't load your folders." action={<Btn onClick={() => { setPhase('loading'); setAttempt((a) => a + 1) }}>Try again</Btn>}>Nothing was changed. The folders you keep stay on this PC.</ErrorBanner></div>
      ) : phase === 'empty' ? (
        <div className="mt-3 rounded-lg border border-line px-4 py-6 text-center"><div className="text-[14px] font-medium text-ink">No folders yet</div><p className="text-[12px] text-ink-3">Single files you keep offline are in Available offline.</p></div>
      ) : phase === 'loading' ? (
        <div className="mt-3 flex flex-col gap-2"><Skeleton className="h-8" /><Skeleton className="h-8" /><Skeleton className="h-8" /></div>
      ) : (
        <>
          <div className="mt-3 max-h-[300px] overflow-y-auto rounded-lg border border-line py-1">
            <Tree nodes={tree} depth={0} openSet={openSet} flip={(id) => setOpenSet((o) => { const n = new Set(o); if (n.has(id)) n.delete(id); else n.add(id); return n })} />
          </div>
          <div className="mt-2 flex gap-4 text-[12px] text-ink-3">
            <span className="flex items-center gap-1.5"><Check state="on" label="Key: kept" onChange={() => undefined} />Kept</span>
            <span className="flex items-center gap-1.5"><Check state="mixed" label="Key: some folders inside kept" onChange={() => undefined} />Some folders inside kept</span>
            <span className="flex items-center gap-1.5"><Check state="off" label="Key: online only" onChange={() => undefined} />Online only</span>
          </div>
          <p className="mt-2 text-[12px] text-ink-3">Unchecking never deletes anything from Beebeeb: downloaded copies leave this PC and the files download again when opened.</p>
          <p className={`mt-1 text-[13px] text-ink ${mono}`} data-testid="keep-footer">{p.changed ? 'After Apply: ' : 'On this PC: '}{keptLine}</p>
        </>
      )}
    </Dialog>
  )
}

export function KeepConfirm({ arg }: { arg?: string }) {
  const { close, open } = useSettings()
  const [writeError, setWriteError] = useState<string | null>(null)
  if (arg === 'changed') presetKeep()
  const s = useKeep()
  const p = plan(s)
  const names = p.leaving.length === 1 ? find(tree, p.leaving[0].id)?.name ?? '' : `${p.leaving.length} folders`
  const leavingBytes = reportedSum(p.leaving, 'bytes')
  const comingBytes = reportedSum(p.coming, 'bytes')
  return (
    <Dialog labelId="keepc-h" title={`Stop keeping ${names} on this PC?`} onCancel={() => open('keep')}
      actions={<><Btn onClick={() => open('keep')}>Cancel</Btn><Btn variant="amber" onClick={() => { void apply(s).then((result) => wrote(result, setWriteError, close)) }}>Apply</Btn></>}>
      {writeError ? <p role="alert" className="mb-2 text-[12px] text-red-text">{writeError}</p> : null}
      <p>{leavingBytes == null ? null : <><span className={mono}>{formatBytes(leavingBytes)}</span> leaves this PC. </>}Nothing is deleted from Beebeeb: the files download again when you open them.</p>
      <p className="mt-2">Changes that have not uploaded yet stay on this PC until they have.</p>
      {p.coming.length ? <p className="mt-2">{comingBytes == null ? null : <><span className={mono}>{formatBytes(comingBytes)}</span> downloads: </>}{p.coming.map((c) => c.name).join(', ')}.</p> : null}
    </Dialog>
  )
}
