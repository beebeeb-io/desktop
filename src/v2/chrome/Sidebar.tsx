// DW-0401's sidebar: Files, Shared, Photos, Backups, Trash, then This PC (Sync history, Available offline);
// the storage meter; the account row with the settings gear (Ctrl+,). Selection is neutral grey, never amber.
// Every item is a link (<a>) so it reads and behaves as navigation.

import type { MouseEvent } from 'react'
import { Icon, type IconName } from '@beebeeb/shared/design'
import type { StorageSummary } from '@beebeeb/shared/drive-ui'
import type { SectionId } from '../routes'

interface Item {
  section: SectionId
  label: string
  icon: IconName
}

const MAIN: Item[] = [
  { section: 'files', label: 'Files', icon: 'folder' },
  { section: 'shared', label: 'Shared', icon: 'users' },
  { section: 'photos', label: 'Photos', icon: 'image' },
  { section: 'backups', label: 'Backups', icon: 'cloud' },
  { section: 'trash', label: 'Trash', icon: 'trash' },
]

const THIS_PC: Item[] = [
  { section: 'sync-history', label: 'Sync history', icon: 'activity' },
  { section: 'available-offline', label: 'Available offline', icon: 'pin' },
]

const GB = 1000 ** 3
const TB = 1000 ** 4

/**
 * One figure of the meter's line, in decimal units (1 GB = 1000³, 1 TB = 1000⁴). Exactly zero is "0 B";
 * otherwise GB until the value reaches 1 TB, then TB. Below 10 keeps one decimal (a trailing ".0" is
 * dropped), 10 and above is a whole number. No MB unit: the meter is billing usage, not a file size.
 */
function storageFigure(bytes: number): string {
  if (bytes === 0) return '0 B'
  const [size, unit]: [number, string] = bytes >= TB ? [TB, 'TB'] : [GB, 'GB']
  const value = bytes / size
  const text = value < 10 ? value.toFixed(1).replace(/\.0$/, '') : String(Math.round(value))
  return `${text} ${unit}`
}

/** The words under the meter: the account's billing usage, in decimal units. */
export const storageLine = (usedBytes: number, quotaBytes: number) => `${storageFigure(usedBytes)} of ${storageFigure(quotaBytes)}`

/**
 * What a row's count shows. Sync history's is the problem badge: "the sidebar's red badge counts the problems"
 * (DW-1301), so none at zero (DW-0401 draws Sync history bare) and red above it. The others are plain counts.
 */
export function rowCount(section: SectionId, count: number | undefined): { text: string; problems: boolean } | null {
  if (count === undefined) return null
  if (section === 'sync-history') return count > 0 ? { text: String(count), problems: true } : null
  return { text: String(count), problems: false }
}

function NavLink({ item, active, onGo, count }: { item: Item; active: boolean; onGo: (s: SectionId) => void; count?: number }) {
  const onClick = (e: MouseEvent<HTMLAnchorElement>) => {
    e.preventDefault()
    onGo(item.section)
  }
  const shown = rowCount(item.section, count)
  return (
    <a
      href={`?section=${item.section}`}
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      className={`flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] ${active ? 'bg-paper-3 font-medium text-ink' : 'text-ink-2 hover:bg-paper-2'}`}
    >
      <Icon name={item.icon} size={16} />
      <span className="min-w-0 flex-1 truncate">{item.label}</span>
      {shown && (shown.problems
        // The problem count is the board's red badge (DW-1301 "a red 3"): a white number on the red fill, a pill.
        ? <span data-badge="problems" className="inline-grid h-[18px] min-w-[18px] place-items-center rounded-full bg-red-fill px-[5px] text-[10.5px] font-semibold leading-none tabular-nums text-white">{shown.text}</span>
        : <span className="font-mono text-[11px] text-ink-3">{shown.text}</span>)}
    </a>
  )
}

export function Sidebar({
  active,
  onGo,
  storage,
  accountName,
  counts,
}: {
  active: SectionId
  onGo: (section: SectionId) => void
  storage: StorageSummary | null
  accountName: string
  /** Item counts shown at the right of a row (DW-0114: zero for a new account). */
  counts?: Partial<Record<SectionId, number>>
}) {
  const pct = storage ? Math.min(100, (storage.usedBytes / storage.quotaBytes) * 100) : 0
  return (
    <nav aria-label="Sections" className="flex w-[232px] flex-none flex-col border-r border-line bg-paper-2 px-2 py-3">
      <div className="flex flex-col gap-0.5">
        {MAIN.map((item) => <NavLink key={item.section} item={item} active={active === item.section} onGo={onGo} count={counts?.[item.section]} />)}
      </div>
      <div className="mt-5 px-2.5 pb-1 text-[11px] font-medium uppercase tracking-wide text-ink-3">This PC</div>
      <div className="flex flex-col gap-0.5">
        {THIS_PC.map((item) => <NavLink key={item.section} item={item} active={active === item.section} onGo={onGo} count={counts?.[item.section]} />)}
      </div>
      <div className="mt-auto px-2.5">
        {storage && (
          <div className="mb-3">
            <div className="h-1.5 overflow-hidden rounded-full bg-paper-3" role="meter" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100} aria-label="Storage used">
              <div className="h-full bg-ink-3" style={{ width: `${pct}%` }} />
            </div>
            <div className="mt-1.5 font-mono text-[11px] text-ink-3">
              {storageLine(storage.usedBytes, storage.quotaBytes)}
            </div>
          </div>
        )}
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-[13px] text-ink-2">{accountName}</span>
          <a
            href="?section=settings"
            onClick={(e) => { e.preventDefault(); onGo('settings') }}
            aria-label="Settings (Ctrl+,)"
            title="Settings (Ctrl+,)"
            className="grid h-7 w-7 place-items-center rounded-md text-ink-2 hover:bg-paper-3"
          >
            <Icon name="settings" size={16} />
          </a>
        </div>
      </div>
    </nav>
  )
}
