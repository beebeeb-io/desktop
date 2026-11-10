// Dev only (batch M2): live state recipes for B03 (global elements, menus, commands dialog) and B04 (Files, folders,
// Backups, uploads). Row ids are the board's (src/v2/source/fixtures/files.ts): i-contract is Contract draft.pdf.
import type { Recipe } from './states'
import { menuItem } from './driver'

const LIST = '[data-list-count] > div[tabindex="0"]'
const prefs = (folder: string, p: object) => ({ 'bb.v2.folder-prefs': { [folder]: p } })
const PANEL = prefs('root', { panel: true })

export const FILES_STATES: Record<string, Recipe> = {
  // ── B03: global elements ──────────────────────────────────────────────────────────────────────────────────────
  // The window's banner (chrome/Banners, mounted with the caption bar, drawn under the Files toolbar): one per scenario.
  'DW-0101': { q: 'section=files&scenario=offline' },
  'DW-0102': { q: 'section=files&scenario=problems' },
  'DW-0103': { q: 'section=files&scenario=nearly-full' },
  'DW-0104': { q: 'section=files&scenario=full' },
  'DW-0105': { q: 'section=files&scenario=signed-out' },
  'DW-0106': { q: 'section=files&scenario=backup-problems' },
  'DW-0107': {
    q: 'section=files', steps: [{ context: 'row:i-contract' }, { click: menuItem('copy-link') }],
    gap: 'Copy link makes the link in Rust (share_link_copy), which a browser has not: no live path here shows "Link copied"',
  },
  'DW-0108': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'Delete' }] },
  // The fixture's first rename fails for the connection (`rename-fails`); Try again goes through.
  'DW-0109': {
    q: 'section=files&scenario=rename-fails',
    steps: [{ click: 'row:i-contract' }, { key: 'F2' }, { fill: 'field:Name', value: 'Contract final.pdf' }, { key: 'Enter', on: 'field:Name' }],
  },
  'DW-0114': { q: 'section=files&scenario=empty' },
  'DW-0118': { q: 'section=files&scenario=listing' },
  'DW-0119': { q: 'section=files', storage: prefs('root', { view: 'grid' }) },
  'DW-0121': { q: 'section=files&scenario=offline', steps: [{ dbl: 'row:i-contract' }] },
  'DW-1230': { window: 'tray', native: 'the notification area and its icons are drawn by Windows; the board draws them for reference' },
  'DW-1231': { window: 'tray', native: 'the b’s menu is a native Windows menu (Tauri’s tray menu, Rust); no web view draws it' },
  'DW-1231b': { window: 'tray', native: 'the b’s menu is a native Windows menu (Tauri’s tray menu, Rust); no web view draws it' },
  'DW-1232': { q: 'section=files', steps: [{ key: 'Ctrl+/' }] },
  'DW-1233': { q: 'section=files', steps: [{ key: 'Ctrl+/' }, { click: '[role=tab]:nth-child(2)' }] },
  'DW-1234': { q: 'section=open&id=o-md', steps: [{ wait: 'btn:Edit' }, { key: 'Ctrl+/' }] },

  // ── B04: Files ────────────────────────────────────────────────────────────────────────────────────────────────
  'DW-0401': { q: 'section=files', storage: PANEL, steps: [{ click: 'row:i-contract' }] },
  // Files' root keeps its details panel open (DW-0401, DW-0403, DW-0412 draw it), so the menu opens over it too.
  'DW-0402': { q: 'section=files', storage: PANEL, steps: [{ context: 'row:i-contract' }] },
  'DW-0403': { q: 'section=files', storage: PANEL, steps: [{ click: 'row:i-budget' }, { ctrl: 'row:i-contract' }] },
  'DW-0404': { q: 'section=files', steps: [{ click: '[data-anchor="sort"]' }] },
  'DW-0405': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'Delete' }] },
  // The copy is made; the pause lets its 3 s "Duplicated" go, so a second Ctrl+D shows as its own message (M3a).
  'DW-0406': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'Ctrl+d' }, { pause: 3300 }] },
  'DW-0407': { q: 'section=files', steps: [{ context: 'row:i-contract' }, { click: menuItem('share') }] },
  // The preview names Service agreement.pdf; with no PDF page renderer here its body is the honest NoPreview (DW-0408b)
  // rather than another file's text or a thumbnail posing as page 1.
  'DW-0408': { q: 'section=files', steps: [{ click: 'row:i-agreement' }, { key: 'Space' }], gap: 'the preview names the PDF and draws no PDF pages yet (no PDF renderer in this build); its body is the NoPreview of DW-0408b' },
  'DW-0408b': { q: 'section=files', steps: [{ click: 'row:i-moodboard' }, { key: 'Space' }] },
  'DW-0409': { q: 'section=files', steps: [{ context: 'row:i-contract' }, { click: menuItem('move') }, { click: '[data-move-folder="f-documents"]' }] },
  'DW-0410': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'F2' }] },
  'DW-0410b': {
    q: 'section=files',
    steps: [{ click: 'row:i-contract' }, { key: 'Ctrl+d' }, { pause: 200 }, { key: 'F2' }, { fill: 'field:Name', value: 'Contract draft.pdf' }, { key: 'Enter', on: 'field:Name' }],
  },
  'DW-0411': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'Alt+Enter' }] },
  'DW-0412': { q: 'section=files', storage: PANEL, steps: [{ context: 'row:i-contract' }, { click: menuItem('offline') }] },
  'DW-0413': { q: 'section=files', steps: [{ context: 'row:i-contract' }, { click: menuItem('versions') }] },
  'DW-0414': { q: 'section=files', steps: [{ click: '[data-anchor="new"]' }] },
  'DW-0415': { q: 'section=files', native: 'Windows draws the file picker (the board draws it for reference)' },
  'DW-0416': { q: 'section=files', native: 'Windows draws the file picker (the board draws it for reference)' },
  'DW-0417': { q: 'section=files', steps: [{ click: '[data-anchor="new"]' }, { click: menuItem('new-folder') }] },
  'DW-0480': { q: 'section=files', native: 'Windows draws the folder picker (the board draws it for reference)' },
  'DW-0481': { q: 'section=files', gap: 'the drop target shows only during a File Explorer drag (Tauri’s drag-drop event); a browser has none' },

  // Folders, Backups and uploads
  // Opened from Files, as a person gets there, so Back (Alt+Left) has somewhere to go.
  'DW-0420': { q: 'section=files', steps: [{ dbl: 'row:f-lisbon' }, { wait: 'text:Day 1' }] },
  'DW-0420b': { q: 'section=files&id=f-archive', steps: [{ scroll: LIST, top: 138 * 36 }] },
  'DW-0421': { q: 'section=files&id=f-lisbon', storage: prefs('f-lisbon', { view: 'grid' }) },
  'DW-0422': { q: 'section=backups' },
  'DW-0423': { q: 'section=backups&id=f-bk-laptop' },
  'DW-0424': { q: 'section=backups&id=f-bk-mail' },
  'DW-0425': { q: 'section=backups&id=f-bk-mail', steps: [{ click: 'row:f-mail-2026' }, { key: 'F2' }] },
  'DW-0426': { q: 'section=files&id=f-lisbon&scenario=uploading' },
  'DW-0427': { q: 'section=files&id=f-lisbon&scenario=uploading', steps: [{ click: '[data-open-uploads]' }] },
  'DW-0429': { q: 'section=files&scenario=conflict' },

  // Browse files
  'DW-1101': { q: 'section=files' },
  'DW-1102': { q: 'section=files', storage: prefs('f-lisbon', { view: 'grid', panel: true }), steps: [{ dbl: 'row:f-lisbon' }, { click: 'row:p-alfama' }] },
  'DW-1103': { q: 'section=files', steps: [{ context: LIST }] },
  // Ruled (item 3): a downloaded photo that is not kept (Alfama.heic in Lisbon trip), so Make available offline is there.
  'DW-1104': { q: 'section=files&id=f-lisbon', steps: [{ context: 'row:p-alfama' }] },
  // Opened from Files (Search names), so Esc's way back is Files, not whatever the tab showed before.
  'DW-1105': { q: 'section=files', steps: [{ click: 'btn:Search names' }, { fill: 'input[type=search],[role=searchbox],input', value: 'Contr' }] },
  'DW-1106': { q: 'section=files&id=f-new' },
  'DW-1107': { q: 'section=files&scenario=listing' },
  'DW-1108': { q: 'section=files&scenario=offline' },
  'DW-1109': { q: 'section=files&scenario=signed-out' },
  'DW-1110': { q: 'section=backups', steps: [{ click: 'row:f-bk-mail' }, { key: 'F2' }] },
}
