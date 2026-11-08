// DW-1180 to DW-1183: Windows draws File Explorer; this is the picture of it on the Beebeeb folder, so the board's
// states can be looked at and checked: the Status column's three icons, the menus, the toast.
import type { ReactNode } from 'react'

type Sync = 'cloud' | 'ring' | 'filled'
interface Item { name: string; kind: 'folder' | 'file'; type: string; size: string; date: string; sync: Sync; lock?: boolean }

export const ITEMS: Item[] = [
  { name: 'Backups', kind: 'folder', type: 'File folder', size: '', date: '8 Nov 2026 09:12', sync: 'filled', lock: true },
  { name: 'Contracts', kind: 'folder', type: 'File folder', size: '', date: '7 Nov 2026 16:40', sync: 'ring' },
  { name: 'Design', kind: 'folder', type: 'File folder', size: '', date: '5 Nov 2026 11:03', sync: 'ring' },
  { name: 'Invoices', kind: 'folder', type: 'File folder', size: '', date: '2 Nov 2026 08:55', sync: 'cloud' },
  { name: 'Photos', kind: 'folder', type: 'File folder', size: '', date: '30 Oct 2026 19:21', sync: 'cloud' },
  { name: 'Brand guide.pdf', kind: 'file', type: 'PDF Document', size: '4.2 MB', date: '28 Oct 2026 14:08', sync: 'cloud' },
  { name: 'Budget 2026.xlsx', kind: 'file', type: 'Excel Worksheet', size: '86 KB', date: '27 Oct 2026 10:30', sync: 'ring' },
  { name: 'Contract draft.pdf', kind: 'file', type: 'PDF Document', size: '1.1 MB', date: '8 Nov 2026 10:38', sync: 'filled' },
  { name: 'Logo.svg', kind: 'file', type: 'SVG Document', size: '12 KB', date: '20 Oct 2026 17:45', sync: 'ring' },
  { name: 'Notes.md', kind: 'file', type: 'Markdown File', size: '3 KB', date: '18 Oct 2026 09:02', sync: 'filled' },
  { name: 'Roadmap.pdf', kind: 'file', type: 'PDF Document', size: '2.4 MB', date: '12 Oct 2026 13:19', sync: 'cloud' },
]

const SYNC_LABEL: Record<Sync, string> = { cloud: 'Online only', ring: 'On this PC', filled: 'Always on this PC' }

function SyncIcon({ sync, lock }: { sync: Sync; lock?: boolean }) {
  const p = { width: 16, height: 16, viewBox: '0 0 16 16', fill: 'none', stroke: 'currentColor', strokeWidth: 1.3, strokeLinecap: 'round', strokeLinejoin: 'round', role: 'img', 'aria-label': SYNC_LABEL[sync] } as const
  return (
    <span className="flex items-center gap-1.5">
      {sync === 'cloud' ? <svg {...p}><path d="M4.5 12.5h6.3a2.6 2.6 0 0 0 .4-5.1A3.6 3.6 0 0 0 4.4 6.6 2.9 2.9 0 0 0 4.5 12.5z" /></svg> : null}
      {sync === 'ring' ? <svg {...p}><circle cx="8" cy="8" r="5.5" /><path d="M5.6 8.1l1.7 1.7 3.1-3.3" /></svg> : null}
      {sync === 'filled' ? <svg {...p}><circle cx="8" cy="8" r="5.5" fill="currentColor" /><path d="M5.6 8.1l1.7 1.7 3.1-3.3" stroke="var(--color-paper)" /></svg> : null}
      {lock ? <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3" role="img" aria-label="Locked"><rect x="3.5" y="7" width="9" height="6.5" rx="1.5" /><path d="M5.5 7V5.2a2.5 2.5 0 0 1 5 0V7" /></svg> : null}
    </span>
  )
}

function Glyph({ kind }: { kind: Item['kind'] }) {
  return kind === 'folder'
    ? <span aria-hidden="true" className="inline-block h-3.5 w-4 rounded-sm bg-amber-bg ring-1 ring-amber-deep" />
    : <span aria-hidden="true" className="inline-block h-4 w-3 rounded-sm border border-ink-4 bg-paper" />
}

function MenuItem({ children, bold, rule }: { children: ReactNode; bold?: boolean; rule?: boolean }) {
  return <li role="menuitem" className={`px-3 py-1 text-[12px] ${bold ? 'font-semibold' : ''} ${rule ? 'mt-1 border-t border-line pt-1.5' : ''}`}>{children}</li>
}

export const MODERN_MENU = ['Open', 'Open with', 'Always keep on this device', 'Free up space', 'Compress to ZIP file', 'Copy as path', 'Properties', 'Show more options']
export const CLASSIC_MENU_TOP = ['Open', 'Open with', 'Always keep on this device', 'Free up space']
export const BEEBEEB_VERBS = ['Share with Beebeeb…', 'Copy Beebeeb link', 'Show in Beebeeb', 'Version history…']
export const CLASSIC_MENU_BOTTOM = ['Give access to', 'Send to', 'Cut', 'Copy', 'Create shortcut', 'Delete', 'Rename', 'Properties']

export const TOAST = { title: 'Beebeeb manages this folder', text: 'Backups and the folders Beebeeb services keep can’t be moved, renamed or deleted. You can open and download what’s inside.' }

export function Explorer({ selected, menu, toast }: { selected: string; menu?: 'modern' | 'classic'; toast?: boolean }) {
  return (
    <div className="relative h-screen w-screen overflow-hidden bg-paper-3 p-6 font-sans text-ink">
      <div className="flex h-full flex-col overflow-hidden rounded-lg border border-line bg-paper shadow-lg" data-explorer="beebeeb">
        <div className="flex h-9 flex-none items-center gap-2 border-b border-line bg-paper-2 px-3 text-[12px]"><span className="font-semibold">Beebeeb</span></div>
        <div className="flex h-10 flex-none items-center gap-3 border-b border-line px-3 text-[12px] text-ink-2">
          <span>New</span><span>Sort</span><span>View</span>
          <span className="ml-3 flex-1 rounded-md border border-line bg-paper-2 px-2 py-1 font-mono text-[11px]">This PC › Beebeeb</span>
        </div>
        <div className="flex min-h-0 flex-1">
          <nav aria-label="Navigation pane" className="w-44 flex-none border-r border-line bg-paper-2 p-2 text-[12px]">
            <ul className="space-y-1">
              <li className="px-2 py-1 text-ink-2">Home</li>
              <li className="px-2 py-1 text-ink-2">Gallery</li>
              <li className="rounded-md bg-paper-3 px-2 py-1 font-semibold" data-nav="beebeeb">Beebeeb</li>
              <li className="px-2 py-1 text-ink-2">Desktop</li>
              <li className="px-2 py-1 text-ink-2">Documents</li>
              <li className="px-2 py-1 text-ink-2">This PC</li>
            </ul>
          </nav>
          <div className="min-w-0 flex-1 overflow-hidden">
            <table className="w-full table-fixed border-collapse text-left text-[12px]">
              <thead className="text-ink-3">
                <tr className="h-8 border-b border-line">
                  <th className="w-[34%] px-3 font-normal">Name</th><th className="w-[20%] font-normal">Status</th><th className="w-[22%] font-normal">Date modified</th><th className="w-[14%] font-normal">Type</th><th className="font-normal">Size</th>
                </tr>
              </thead>
              <tbody>
                {ITEMS.map((it) => (
                  <tr key={it.name} data-row="item" aria-selected={it.name === selected} className={`h-8 ${it.name === selected ? 'bg-paper-3' : ''}`}>
                    <td className="truncate px-3"><span className="mr-2 inline-flex align-middle"><Glyph kind={it.kind} /></span>{it.name}</td>
                    <td className="text-ink-2"><SyncIcon sync={it.sync} lock={it.lock} /></td>
                    <td className="font-mono text-[11px] text-ink-3">{it.date}</td>
                    <td className="truncate text-ink-3">{it.type}</td>
                    <td className="font-mono text-[11px] text-ink-3">{it.size}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
        <div role="status" className="flex h-7 flex-none items-center gap-4 border-t border-line bg-paper-2 px-3 text-[12px] text-ink-3">
          <span>11 items</span><span>1 item selected</span><span className="ml-auto">Beebeeb: up to date</span>
        </div>
      </div>
      {menu ? (
        <ul role="menu" aria-label={menu === 'modern' ? 'File Explorer menu' : 'Classic menu'} data-menu={menu} className="absolute left-72 top-40 w-60 rounded-lg border border-line bg-paper py-1.5 shadow-lg">
          {menu === 'modern'
            ? MODERN_MENU.map((m, i) => <MenuItem key={m} bold={i === 0} rule={m === 'Compress to ZIP file' || m === 'Show more options'}>{m}</MenuItem>)
            : (<>
              {CLASSIC_MENU_TOP.map((m, i) => <MenuItem key={m} bold={i === 0}>{m}</MenuItem>)}
              {BEEBEEB_VERBS.map((m, i) => <MenuItem key={m} rule={i === 0}>{m}</MenuItem>)}
              {CLASSIC_MENU_BOTTOM.map((m, i) => <MenuItem key={m} rule={i === 0 || m === 'Cut' || m === 'Delete'}>{m}</MenuItem>)}
            </>)}
        </ul>
      ) : null}
      {toast ? (
        <aside role="alert" data-toast="beebeeb" className="absolute bottom-14 right-6 w-[360px] rounded-lg border border-line bg-paper p-4 shadow-lg">
          <p className="text-[13px] font-semibold">{TOAST.title}</p>
          <p className="mt-1 text-[12px] leading-4 text-ink-3">{TOAST.text}</p>
        </aside>
      ) : null}
    </div>
  )
}
