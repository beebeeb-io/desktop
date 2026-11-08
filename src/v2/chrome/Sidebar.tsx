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

const GB = 1024 ** 3
const gb = (bytes: number) => (bytes >= 1024 * GB ? `${(bytes / (1024 * GB)).toFixed(0)} TB` : `${Math.round(bytes / GB)} GB`)

function NavLink({ item, active, onGo }: { item: Item; active: boolean; onGo: (s: SectionId) => void }) {
  const onClick = (e: MouseEvent<HTMLAnchorElement>) => {
    e.preventDefault()
    onGo(item.section)
  }
  return (
    <a
      href={`?section=${item.section}`}
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      className={`flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] ${active ? 'bg-paper-3 font-medium text-ink' : 'text-ink-2 hover:bg-paper-2'}`}
    >
      <Icon name={item.icon} size={16} />
      <span className="truncate">{item.label}</span>
    </a>
  )
}

export function Sidebar({
  active,
  onGo,
  storage,
  accountName,
}: {
  active: SectionId
  onGo: (section: SectionId) => void
  storage: StorageSummary | null
  accountName: string
}) {
  const pct = storage ? Math.min(100, (storage.usedBytes / storage.quotaBytes) * 100) : 0
  return (
    <nav aria-label="Sections" className="flex w-[232px] flex-none flex-col border-r border-line bg-paper-2 px-2 py-3">
      <div className="flex flex-col gap-0.5">
        {MAIN.map((item) => <NavLink key={item.section} item={item} active={active === item.section} onGo={onGo} />)}
      </div>
      <div className="mt-5 px-2.5 pb-1 text-[11px] font-medium uppercase tracking-wide text-ink-3">This PC</div>
      <div className="flex flex-col gap-0.5">
        {THIS_PC.map((item) => <NavLink key={item.section} item={item} active={active === item.section} onGo={onGo} />)}
      </div>
      <div className="mt-auto px-2.5">
        {storage && (
          <div className="mb-3">
            <div className="h-1.5 overflow-hidden rounded-full bg-paper-3" role="meter" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100} aria-label="Storage used">
              <div className="h-full bg-ink-3" style={{ width: `${pct}%` }} />
            </div>
            <div className="mt-1.5 font-mono text-[11px] text-ink-3">
              {gb(storage.usedBytes)} of {gb(storage.quotaBytes)}
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
