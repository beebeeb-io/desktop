// Keep on this PC (DW-0784 family): choose which folders stay on this PC, many at once, and see what each choice
// does before it happens. Checked = kept (everything inside, also what is added later); nothing changes until Apply.
import { useState, useSyncExternalStore } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { command } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { Btn, Dialog, mono } from '../shell/kit'
import { useSettings } from '../shell/context'
import { KEEP_AFTER, KEEP_BEFORE, KEEP_TREE, type KeepNode } from '../../../source/fixtures/settings-today'
import { Check, ErrorBanner, Skeleton } from './ui'

interface KeepStore { checked: Set<string>; initial: Set<string> }
let store: KeepStore = { checked: new Set(KEEP_BEFORE), initial: new Set(KEEP_BEFORE) }
const subs = new Set<() => void>()
const put = (s: KeepStore) => { store = s; subs.forEach((f) => f()) }
const useKeep = () => useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f) } }, () => store)
export const resetKeep = () => put({ checked: new Set(KEEP_BEFORE), initial: new Set(KEEP_BEFORE) })
export const presetKeep = () => put({ checked: new Set(KEEP_AFTER), initial: new Set(KEEP_BEFORE) })

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
const leafOn = (leaf: KeepNode, set: Set<string>, nodes = KEEP_TREE, inherited = false): boolean => {
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
  if (stateOf(n, set, ancestorOn(n.id, set)) === 'on') uncheck(KEEP_TREE, n.id, set, false)
  else { ids(n).forEach((i) => set.delete(i)); set.add(n.id) }
  put({ ...store, checked: set })
}
const ancestorOn = (id: string, set: Set<string>, nodes = KEEP_TREE, inherited = false): boolean => {
  for (const n of nodes) {
    const on = inherited || set.has(n.id)
    if (n.id === id) return inherited
    if (n.children && contains(n, id)) return ancestorOn(id, set, n.children, on)
  }
  return false
}

const gb = (v: number) => (v < 0.1 ? `${Math.round(v * 1024)} MB` : `${v.toFixed(1)} GB`)
export function plan(s: KeepStore) {
  const ls = leaves(KEEP_TREE)
  const was = (l: KeepNode) => leafOn(l, s.initial)
  const now = (l: KeepNode) => leafOn(l, s.checked)
  const leaving = ls.filter((l) => was(l) && !now(l))
  const coming = ls.filter((l) => !was(l) && now(l))
  const on = ls.filter(now).reduce((t, l) => t + l.gb, 0)
  const all = ls.reduce((t, l) => t + l.gb, 0)
  return { leaving, coming, leaveGb: leaving.reduce((t, l) => t + l.gb, 0), comeGb: coming.reduce((t, l) => t + l.gb, 0), on, off: all - on, changed: leaving.length + coming.length > 0 }
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
                {leaving ? `Leaves this PC · ${gb(n.gb)}` : coming ? `Downloads · ${gb(n.gb)}` : gb(n.gb)}
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

async function apply(s: KeepStore) {
  if (!hasTauri()) return
  const excluded = leaves(KEEP_TREE).filter((l) => !leafOn(l, s.checked)).map((l) => l.id)
  await command('set_selective_sync', { excluded })
}

export function KeepDialog({ arg }: { arg?: string }) {
  const { close, open } = useSettings()
  useState(() => { if (arg === 'changed') presetKeep(); else if (!arg) resetKeep() })
  const s = useKeep()
  const [openSet, setOpenSet] = useState(new Set(['lisbon']))
  const [phase, setPhase] = useState(arg === 'error' ? 'error' : arg === 'empty' ? 'empty' : arg === 'loading' ? 'loading' : 'ready')
  const p = plan(s)
  const go = () => { if (p.leaving.length) open('keep-confirm'); else { void apply(s); close() } }
  return (
    <Dialog labelId="keep-h" title="Keep on this PC" wide onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={go} disabled={!p.changed || phase !== 'ready'}>Apply</Btn></>}>
      <p>Folders you choose stay on this PC. Everything else downloads when you open it.</p>
      {phase === 'error' ? (
        <div className="mt-3"><ErrorBanner title="Couldn't load your folders." action={<Btn onClick={() => setPhase('loading')}>Try again</Btn>}>Nothing was changed. The folders you keep stay on this PC.</ErrorBanner></div>
      ) : phase === 'empty' ? (
        <div className="mt-3 rounded-lg border border-line px-4 py-6 text-center"><div className="text-[14px] font-medium text-ink">No folders yet</div><p className="text-[12px] text-ink-3">Single files you keep offline are in Available offline.</p></div>
      ) : phase === 'loading' ? (
        <div className="mt-3 flex flex-col gap-2"><Skeleton className="h-8" /><Skeleton className="h-8" /><Skeleton className="h-8" /></div>
      ) : (
        <>
          <div className="mt-3 max-h-[300px] overflow-y-auto rounded-lg border border-line py-1">
            <Tree nodes={KEEP_TREE} depth={0} openSet={openSet} flip={(id) => setOpenSet((o) => { const n = new Set(o); if (n.has(id)) n.delete(id); else n.add(id); return n })} />
          </div>
          <div className="mt-2 flex gap-4 text-[12px] text-ink-3">
            <span className="flex items-center gap-1.5"><Check state="on" label="Key: kept" onChange={() => undefined} />Kept</span>
            <span className="flex items-center gap-1.5"><Check state="mixed" label="Key: some folders inside kept" onChange={() => undefined} />Some folders inside kept</span>
            <span className="flex items-center gap-1.5"><Check state="off" label="Key: online only" onChange={() => undefined} />Online only</span>
          </div>
          <p className="mt-2 text-[12px] text-ink-3">Unchecking never deletes anything from Beebeeb: downloaded copies leave this PC and the files download again when opened.</p>
          <p className={`mt-1 text-[13px] text-ink ${mono}`} data-testid="keep-footer">{p.changed ? 'After Apply: ' : 'On this PC: '}{gb(p.on)} on this PC · {gb(p.off)} online only</p>
        </>
      )}
    </Dialog>
  )
}

export function KeepConfirm({ arg }: { arg?: string }) {
  const { close, open } = useSettings()
  useState(() => { if (arg === 'changed') presetKeep() })
  const s = useKeep()
  const p = plan(s)
  const names = p.leaving.length === 1 ? find(KEEP_TREE, p.leaving[0].id)?.name ?? '' : `${p.leaving.length} folders`
  return (
    <Dialog labelId="keepc-h" title={`Stop keeping ${names} on this PC?`} onCancel={() => open('keep')}
      actions={<><Btn onClick={() => open('keep')}>Cancel</Btn><Btn variant="amber" onClick={() => { void apply(s); close() }}>Apply</Btn></>}>
      <p><span className={mono}>{gb(p.leaveGb)}</span> leaves this PC. Nothing is deleted from Beebeeb: the files download again when you open them.</p>
      <p className="mt-2">Changes that have not uploaded yet stay on this PC until they have.</p>
      {p.coming.length ? <p className="mt-2"><span className={mono}>{gb(p.comeGb)}</span> downloads: {p.coming.map((c) => c.name).join(', ')}.</p> : null}
    </Dialog>
  )
}
