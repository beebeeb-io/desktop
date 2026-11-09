// Search, live: DriveSource.search over the list on this PC, with every hit's folder path in one drive_paths call (B02b). Recent
// searches stay on this PC (localStorage) and Clear removes them. Esc goes back to the view you came from.
import { useEffect, useMemo, useRef, useState } from 'react'
import { useDriveSource, type DriveItem } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { Menu, type MenuEntry } from '../files/ui'
import { fileMenu } from '../files/menus'
import { showToast } from '../../chrome/Toast'
import { SearchView, type Pill, type SearchHit } from './SearchView'
import { useDriveExtras } from '../../source/extras'
import { openFailureText } from '../shared/api'

const RECENT_KEY = 'bb.v2.search.recent'
const MAX_RECENT = 8
const MAX_WHERE = 500

function loadRecent(): string[] {
  try { const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]'); return Array.isArray(v) ? v.filter((x) => typeof x === 'string').slice(0, MAX_RECENT) : [] } catch { return [] }
}
function saveRecent(list: string[]) { try { localStorage.setItem(RECENT_KEY, JSON.stringify(list)) } catch { /* private mode: recent searches are not kept */ } }

/** DW-0802: the usual file menu, plus Show in enclosing folder. */
export function searchMenu(item: DriveItem): MenuEntry[] {
  return [{ id: 'enclosing', label: 'Show in enclosing folder', dividerAfter: true }, ...fileMenu([item])]
}

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [query, setQuery] = useState(nav.route.sub ?? '')
  const [pill, setPill] = useState<Pill>('All')
  const [hits, setHits] = useState<SearchHit[]>([])
  const [recent, setRecent] = useState<string[]>(loadRecent)
  const [menu, setMenu] = useState<{ item: DriveItem; x: number; y: number } | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const seq = useRef(0)
  const now = useMemo(() => new Date(), [])
  // The first sign-in's list check (B02b drive_status.listing): results may be incomplete while it runs.
  const [loading, setLoading] = useState<{ done: number; total: number } | undefined>(undefined)
  useEffect(() => { extras.status().then((s) => setLoading(s.listing), () => setLoading(undefined)) }, [extras, query])

  useEffect(() => {
    const q = query.trim()
    const mine = ++seq.current
    setSelected(null)
    if (!q) { setHits([]); return }
    void (async () => {
      const items = await source.search(q).catch(() => [] as DriveItem[])
      if (mine !== seq.current) return
      setHits(items.map((item) => ({ item, where: '' })))
      // Where each one is: the folder path the list on this PC already has.
      const wheres = await extras.paths(items.slice(0, MAX_WHERE).map((i) => i.id)).catch(() => ({} as Record<string, string>))
      if (mine !== seq.current) return
      setHits(items.map((item) => ({ item, where: wheres[item.id] ?? '' })))
    })()
  }, [query, source, extras])

  const remember = (q: string) => {
    const t = q.trim()
    if (!t) return
    const next = [t, ...recent.filter((r) => r.toLowerCase() !== t.toLowerCase())].slice(0, MAX_RECENT)
    setRecent(next)
    saveRecent(next)
  }
  const open = (item: DriveItem) => {
    remember(query)
    if (item.isFolder) nav.go({ section: 'files', id: item.id })
    else void source.open(item.id).catch((e: unknown) => showToast({ kind: 'failed', text: openFailureText(e, 'open it') }))
  }
  const pick = (id: string, item: DriveItem) => {
    remember(query)
    if (id === 'open') return open(item)
    if (id === 'reveal') return void source.revealInFileManager(item.id).catch((e: unknown) => showToast({ kind: 'failed', text: openFailureText(e, 'show it in File Explorer') }))
    if (id === 'offline') {
      // The same switch and words as Files' (DW-0412): a toast either way, never a silent failure.
      const on = item.local !== 'available-offline'
      return void source.setAvailableOffline(item.id, on).then(
        () => showToast({ kind: 'done', text: on ? 'Keeping on this PC' : 'Removed from this PC' }),
        (e: unknown) => showToast({ kind: 'failed', text: openFailureText(e, on ? 'keep it on this PC' : 'remove it from this PC') }))
    }
    // Everything else acts in the item's own folder, where Files shows it selected.
    nav.go({ section: 'files', id: item.parentId, sub: id === 'enclosing' ? `select:${item.id}` : `${id}:${item.id}` })
  }

  return (
    <>
      <SearchView
        query={query} hits={hits} pill={pill} recent={recent} now={now} menuOn={menu?.item.id} loading={loading} selected={selected}
        onQuery={setQuery} onPill={setPill} onOpen={open} onSelect={setSelected}
        onContext={(item, x, y) => setMenu({ item, x, y })}
        onClearRecent={() => { setRecent([]); saveRecent([]) }}
        onEnd={() => nav.back()}
      />
      {menu && <Menu x={menu.x} y={menu.y} entries={searchMenu(menu.item)} label="Result" onPick={(id) => pick(id, menu.item)} onClose={() => setMenu(null)} />}
    </>
  )
}
