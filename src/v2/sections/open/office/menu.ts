// Office files open in the editor by default; Open With offers another app (DW-1562). The Files right-click menu
// calls `withOfficeOpen` on its entries. `apps` are the apps Windows reports for the type (none listed yet: open item).
import type { DriveItem } from '@beebeeb/shared/drive-ui'
import type { MenuEntry } from '../../files/ui'
import { isOfficeFile } from './index'

export const OPEN_WITH_EDITOR = 'open-with:beebeeb'
export const OPEN_WITH_OTHER = 'open-with:other'

export function withOfficeOpen(entries: MenuEntry[], items: DriveItem[], apps: string[] = [], showSubmenu = false): MenuEntry[] {
  if (items.length !== 1 || items[0].isFolder || !isOfficeFile(items[0])) return entries
  return entries.flatMap((e) => e.id !== 'open' ? [e] : [
    { ...e, label: 'Open (Beebeeb editor)' },
    {
      id: 'open-with', label: 'Open With', open: showSubmenu || undefined,
      submenu: [
        { id: OPEN_WITH_EDITOR, label: 'Beebeeb editor (default)', checked: true, dividerAfter: true },
        ...apps.map((a) => ({ id: `open-with:app:${a}`, label: a })),
        { id: OPEN_WITH_OTHER, label: 'Other…' },
      ],
    },
  ])
}
