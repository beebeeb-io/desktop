// Storage › Storage insights (DW-0783 family): where the storage goes, by type, and the files that take the most.
import { useEffect, useState } from 'react'
import { Btn, Group, Meter, Page, Row, fmtBytes, mono } from '../shell/kit'
import { INSIGHTS, type Kind } from '../../../source/fixtures/settings-today'
import { BackLink, ErrorBanner, Skeleton } from './ui'

/** Cut a path in the middle so the file name stays readable. */
export function cutPath(path: string, max = 44): string {
  if (path.length <= max) return path
  const parts = path.split('/')
  const name = parts[parts.length - 1]
  const head = parts[0]
  const room = Math.max(4, max - name.length - head.length - 5)
  return name.length + head.length + 5 > max ? `…/${name}` : `${head}/…/${parts.slice(-2, -1)[0].slice(-room)}/${name}`
}

export type InsightsState = 'ready' | 'loading' | 'error' | 'empty'

export function Insights({ state: initial = 'ready' }: { state?: InsightsState } = {}) {
  const [state, setState] = useState<InsightsState>(initial)
  const [menu, setMenu] = useState<{ i: number; x: number; y: number } | null>(null)
  useEffect(() => {
    if (!menu) return
    const off = () => setMenu(null)
    document.addEventListener('mousedown', off)
    return () => document.removeEventListener('mousedown', off)
  }, [menu])
  const d = INSIGHTS
  const heading = <BackLink to="storage">Storage</BackLink>
  if (state === 'loading') {
    return (
      <Page title="Storage insights" back={heading}>
        {[0, 1, 2].map((i) => <Skeleton key={i} className="h-[120px] w-full" />)}
      </Page>
    )
  }
  if (state === 'error') {
    return (
      <Page title="Storage insights" back={heading}>
        <ErrorBanner title="Couldn't load your insights" action={<Btn onClick={() => setState('loading')}>Try again</Btn>}>
          <span className={`text-[12px] ${mono}`}>The file list didn't answer in time.</span>
        </ErrorBanner>
      </Page>
    )
  }
  if (state === 'empty') {
    return (
      <Page title="Storage insights" back={heading}>
        <Group><div className="px-4 py-6 text-center"><div className="text-[14px] font-medium text-ink">Nothing is stored yet</div><p className="text-[13px] text-ink-3">When you add files, this page shows where the storage goes, by type, and which files take the most.</p></div></Group>
      </Page>
    )
  }
  const maxBytes = Math.max(...d.byType.map((t) => t.bytes))
  const shade: Record<Kind, string> = { Media: 'bg-ink', Documents: 'bg-ink-3', Other: 'bg-ink-4' }
  return (
    <Page title="Storage insights" back={heading}>
      <Group title="Encrypted storage">
        <div className="px-4 py-3">
          <div className="mb-1.5 flex items-baseline justify-between"><span className={`text-[13px] text-ink ${mono}`}>{fmtBytes(d.usedBytes)} of {fmtBytes(d.quotaBytes)}</span><span className="text-[12px] text-ink-3">{d.plan} plan</span></div>
          <Meter label="Encrypted storage used" fraction={d.usedBytes / d.quotaBytes} amber />
          <div className={`mt-1.5 flex justify-between text-[12px] text-ink-3 ${mono}`}><span>{Math.round((d.usedBytes / d.quotaBytes) * 100)}% used</span><span>{fmtBytes(d.quotaBytes - d.usedBytes)} free</span></div>
        </div>
      </Group>
      <Group title="By type" note="Counts and sizes include Backups and Trash.">
        <div className={`px-4 pt-3 text-[12px] text-ink-3 ${mono}`}>{d.files.toLocaleString('en')} files · {fmtBytes(d.usedBytes)}</div>
        {d.byType.map((t) => (
          <div key={t.kind} className="px-4 py-2.5" data-testid="type-row">
            <div className="mb-1 flex justify-between text-[13px]"><span className="text-ink">{t.kind}</span><span className={`text-ink-2 ${mono}`}>{fmtBytes(t.bytes)} · {t.files.toLocaleString('en')} files</span></div>
            <div role="progressbar" aria-label={`${t.kind} share`} aria-valuenow={Math.round((t.bytes / maxBytes) * 100)} className="h-1.5 overflow-hidden rounded-full bg-paper-3">
              <div className={`h-full rounded-full ${t.bytes === maxBytes ? shade.Media : shade[t.kind]}`} style={{ width: `${(t.bytes / maxBytes) * 100}%` }} />
            </div>
          </div>
        ))}
      </Group>
      <Group title={`Largest files, top ${d.largestTotal}`} note="Worked out on this PC from the decrypted file list; Beebeeb's servers never see names or kinds.">
        {d.largest.map((f, i) => (
          <a key={f.path} href={`?section=files&select=${encodeURIComponent(f.path)}`} title={f.path} data-testid="largest-row"
            onContextMenu={(e) => { e.preventDefault(); setMenu({ i, x: e.clientX, y: e.clientY }) }}
            className="flex min-h-[40px] items-center gap-3 px-4 py-1.5 hover:bg-paper-2">
            <span className={`min-w-0 flex-1 truncate text-[12px] text-ink ${mono}`}>{cutPath(f.path)}</span>
            <span className="w-16 shrink-0 text-[12px] text-ink-3">{f.kind}</span>
            <span className={`w-16 shrink-0 text-right text-[12px] text-ink-2 ${mono}`}>{fmtBytes(f.bytes)}</span>
          </a>
        ))}
        <Row label={<span className="text-ink-3">and {d.largestTotal - d.largest.length} more</span>} />
      </Group>
      {menu ? (
        <ul role="menu" className="fixed z-50 min-w-[190px] rounded-lg border border-line bg-paper py-1 shadow-lg" style={{ left: menu.x, top: menu.y }}>
          {['Show in File Explorer', 'Move to Trash'].map((l) => <li key={l}><button role="menuitem" type="button" onMouseDown={(e) => e.stopPropagation()} onClick={() => setMenu(null)} className="w-full px-3 py-1.5 text-left text-[13px] text-ink hover:bg-paper-2">{l}</button></li>)}
        </ul>
      ) : null}
    </Page>
  )
}
