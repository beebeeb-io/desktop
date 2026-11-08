// Dev only (batch M2): live state recipes for B03 (global elements, menus, commands dialog) and B04 (Files, folders,
// Backups, uploads). Row ids are the board's (src/v2/source/fixtures/files.ts): i-contract is Contract draft.pdf.
import type { Recipe } from './states'

const LIST = '[data-list-count] > div[tabindex="0"]'
const prefs = (folder: string, p: object) => ({ 'bb.v2.folder-prefs': { [folder]: p } })
const PANEL = prefs('root', { panel: true })
const NO_BANNERS = 'the window draws no banners: chrome/Banners is not mounted in V2Shell or any section'
const menuItem = (id: string) => `[data-menu-item="${id}"]`

export const FILES_STATES: Record<string, Recipe> = {
  // ── B03: global elements ──────────────────────────────────────────────────────────────────────────────────────
  'DW-0101': { q: 'section=files&scenario=offline', gap: `the offline banner: ${NO_BANNERS}` },
  'DW-0102': { q: 'section=files&scenario=problems', gap: `the sync-problem banner: ${NO_BANNERS}` },
  'DW-0103': { q: 'section=files&scenario=nearly-full', gap: `the storage-almost-full banner: ${NO_BANNERS}` },
  'DW-0104': { q: 'section=files&scenario=full', gap: `the storage-full banner: ${NO_BANNERS}` },
  'DW-0105': { q: 'section=files', gap: `the signed-out banner: ${NO_BANNERS}` },
  'DW-0106': { q: 'section=files', gap: `the backup-problem banner: ${NO_BANNERS}` },
  'DW-0107': {
    q: 'section=files', steps: [{ context: 'row:i-contract' }, { click: menuItem('copy-link') }],
    gap: 'Copy link in Files answers "Sharing isn’t available in this build yet": no live path in Files shows "Link copied"',
  },
  'DW-0108': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'Delete' }] },
  'DW-0109': { q: 'section=files', gap: 'a failed rename never reaches the fixture source (it has no offline write); the toast only shows on a host error' },
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
  'DW-0402': { q: 'section=files', steps: [{ context: 'row:i-contract' }] },
  'DW-0403': { q: 'section=files', storage: PANEL, steps: [{ click: 'row:i-budget' }, { ctrl: 'row:i-contract' }] },
  'DW-0404': { q: 'section=files', steps: [{ click: '[data-anchor="sort"]' }] },
  'DW-0405': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'Delete' }] },
  'DW-0406': { q: 'section=files', steps: [{ click: 'row:i-contract' }, { key: 'Ctrl+d' }] },
  'DW-0407': { q: 'section=files', steps: [{ context: 'row:i-contract' }, { click: menuItem('share') }] },
  // The live preview shows text and thumbnails; Meeting notes.md stands in for the board's PDF page (same panel).
  'DW-0408': { q: 'section=files', steps: [{ click: 'row:i-notes' }, { key: 'Space' }], gap: 'the preview panel draws no PDF pages (PreviewHost previews text and thumbnails); Meeting notes.md opens the same panel' },
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
  'DW-0420': { q: 'section=files&id=f-lisbon' },
  'DW-0420b': { q: 'section=files&id=f-archive', steps: [{ scroll: LIST, top: 138 * 36 }] },
  'DW-0421': { q: 'section=files&id=f-lisbon', storage: prefs('f-lisbon', { view: 'grid' }) },
  'DW-0422': { q: 'section=backups' },
  'DW-0423': { q: 'section=backups&id=f-bk-laptop' },
  'DW-0424': { q: 'section=backups&id=f-bk-mail' },
  'DW-0425': { q: 'section=backups&id=f-bk-mail', steps: [{ click: 'row:f-mail-2026' }, { key: 'F2' }] },
  'DW-0426': { q: 'section=files&id=f-lisbon' },
  'DW-0427': { q: 'section=files&id=f-lisbon', steps: [{ click: '[data-open-uploads]' }] },
  'DW-0429': { q: 'section=files', gap: 'the conflict dialog opens from drive_status conflicts; the fixture extras report none' },

  // Browse files
  'DW-1101': { q: 'section=files' },
  'DW-1102': { q: 'section=files&id=f-lisbon', storage: prefs('f-lisbon', { view: 'grid', panel: true }), steps: [{ click: 'row:p-alfama' }] },
  'DW-1103': { q: 'section=files', steps: [{ context: LIST }] },
  'DW-1104': { q: 'section=files', steps: [{ context: 'row:i-harbour' }] },
  'DW-1105': { q: 'section=search', steps: [{ fill: 'input[type=search],[role=searchbox],input', value: 'Contr' }] },
  'DW-1106': { q: 'section=files&id=f-new' },
  'DW-1107': { q: 'section=files&scenario=listing' },
  'DW-1108': { q: 'section=files&scenario=offline', gap: `the offline banner: ${NO_BANNERS}` },
  'DW-1109': { q: 'section=files', gap: `the signed-out banner: ${NO_BANNERS}` },
  'DW-1110': { q: 'section=backups', steps: [{ click: 'row:f-bk-mail' }, { key: 'F2' }] },
}
