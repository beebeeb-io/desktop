// Dev only (batch M2): live state recipes for B05 (Shared, Search, Trash), B06 (Photos, Sync history, Available
// offline), B07 (Open files, the editor) and B08 (Office). Ids are the board's fixtures (src/v2/source/fixtures/).
import type { Recipe } from './states'
import { LISBON_PEOPLE, SAFETY_NUMBER } from '../source/fixtures/shared'
import { SCENARIOS } from '../tray/fixtures'
import { menuItem } from './driver'

const JONAS = LISBON_PEOPLE[1]
const NO_LISTS = { share_list: { invitations: [], links: [], requests: [], pending: [] } }
const peopleOf = (): Recipe['steps'] => [{ context: 'row:sb-lisbon' }, { click: menuItem('people') }, { wait: '[data-people]' }]
const SEARCH = 'field:Search names'
const OPEN_SEARCH = 'btn:Search names'
const OFFICE_ENGINE = 'the Office editor runs only in the Tauri build: Rust (office_v2) serves the engine over the bboffice scheme; a browser gets "We couldn’t prepare the editor"'

export const DRIVE_STATES: Record<string, Recipe> = {
  // ── B05: Shared ───────────────────────────────────────────────────────────────────────────────────────────────
  'DW-0501': { q: 'section=shared&sub=with-me' },
  'DW-0502': { q: 'section=shared&sub=by-me' },
  'DW-0503': { q: 'section=shared&sub=requests' },
  'DW-0504': { q: 'section=shared&sub=requests', steps: [{ click: 'btn:New request' }] },
  'DW-0505': { q: 'section=shared&sub=with-me', steps: [{ context: 'row:sw-brand' }] },
  'DW-0506': { q: 'section=shared&sub=by-me', steps: [{ context: 'row:sl-budget' }] },
  'DW-0507': { q: 'section=shared&sub=requests', steps: [{ context: 'row:rq-selects' }] },
  // Nothing shared yet, but the account has its files (`no-shares`): DW-0509's Files goes to DW-0401's list, not an empty one.
  'DW-0508': { q: 'section=shared&sub=with-me&scenario=no-shares', reads: NO_LISTS },
  'DW-0509': { q: 'section=shared&sub=by-me&scenario=no-shares', reads: NO_LISTS },
  'DW-0510': { q: 'section=shared&sub=requests&scenario=no-shares', reads: NO_LISTS },
  'DW-0511': { q: 'section=shared&sub=by-me', steps: peopleOf() },
  'DW-0512': { q: 'section=shared&sub=by-me', steps: [...peopleOf()!, { click: '[data-person="m-jonas"]' }] },
  'DW-0512b': { q: 'section=shared&sub=by-me', steps: [...peopleOf()!, { click: '[data-person="m-jonas"]' }, { click: 'btn:Remove access' }] },
  'DW-0513': { q: 'section=shared&sub=by-me', steps: [...peopleOf()!, { click: '[data-person="m-jonas"]' }, { click: 'btn:Safety number' }] },
  // "They don't match" is a host write (share_mark_compared): the live state opens on its answer.
  'DW-0513b': {
    q: 'section=shared&sub=by-me', seeds: { 'shared.members': LISBON_PEOPLE, 'shared.dialog': { kind: 'mismatch', itemId: 'sb-lisbon', member: JONAS, groups: SAFETY_NUMBER } },
    steps: [{ wait: '[data-groups]' }],
  },
  'DW-0514': { q: 'section=shared&sub=by-me', steps: [...peopleOf()!, { click: 'btn:Invite someone…' }, { fill: 'field:Email', value: JONAS.email }] },
  // The key-changed alert follows the host's answer to share_invite (a write).
  'DW-0515': {
    q: 'section=shared&sub=by-me',
    seeds: { 'shared.members': LISBON_PEOPLE, 'shared.dialog': { kind: 'key-changed', itemId: 'sb-lisbon', member: JONAS, email: JONAS.email, canEdit: true } },
    steps: [{ wait: 'btn:Share anyway' }],
  },

  // ── B05: Search and Trash ─────────────────────────────────────────────────────────────────────────────────────
  // Search opens from Files (Ctrl+F, the toolbar's Search names), so Esc and × have a view to go back to (M3c).
  'DW-0801': { q: 'section=files', storage: { 'bb.v2.search.recent': ['contract', 'lisbon', 'invoice 2026'] }, steps: [{ click: OPEN_SEARCH }, { wait: SEARCH }] },
  'DW-0802': { q: 'section=files', steps: [{ click: OPEN_SEARCH }, { fill: SEARCH, value: 'Contr' }] },
  'DW-0802b': { q: 'section=files&scenario=listing', steps: [{ click: OPEN_SEARCH }, { fill: SEARCH, value: 'Contr' }] },
  'DW-0803': { q: 'section=files', steps: [{ click: OPEN_SEARCH }, { fill: SEARCH, value: 'harbr' }] },
  'DW-0747': { q: 'section=trash' },
  'DW-0748': { q: 'section=trash', steps: [{ context: 'row:t-contract-old' }] },
  'DW-0749': { q: 'section=trash', steps: [{ click: 'btn:Empty Trash…' }] },
  // The note shows while Rust deletes item by item (drive_delete_permanently, a write).
  'DW-0749b': { q: 'section=trash', seeds: { 'trash.emptying': { done: 5, total: 12 } }, steps: [{ wait: '[aria-label="Emptying Trash"]' }] },
  'DW-0117': { q: 'section=trash&scenario=empty' },

  // ── B06: Photos ───────────────────────────────────────────────────────────────────────────────────────────────
  'DW-0601': { q: 'section=photos' },
  'DW-0602': { q: 'section=photos', steps: [{ click: 'btn:Select' }, { click: 'row:ph-harbour' }, { click: 'row:ph-cafe' }, { click: 'row:ph-sun-4' }] },
  'DW-0603': { q: 'section=photos', steps: [{ click: '[data-anchor="photos-across"]' }] },
  'DW-0604': { q: 'section=photos', steps: [{ click: 'btn:Photo sources' }] },
  'DW-0605': { q: 'section=photos', steps: [{ dbl: 'row:ph-harbour' }] },
  'DW-0605b': { q: 'section=photos', steps: [{ dbl: 'row:ph-harbour' }, { click: 'btn:Original' }], gap: 'the original’s download progress comes from no source: the viewer switches to Original with no bar' },
  'DW-0606': { q: 'section=photos', steps: [{ dbl: 'row:ph-harbour' }, { click: 'btn:More actions' }] },
  'DW-0607': { q: 'section=photos', steps: [{ dbl: 'row:ph-harbour' }, { click: '[data-action="Share"]' }] },
  'DW-0608': { q: 'section=photos', steps: [{ dbl: 'row:ph-harbour' }, { click: '[data-action="Offline"]' }] },
  'DW-0609': { q: 'section=photos', steps: [{ dbl: 'row:ph-harbour' }, { click: '[data-action="Details"]' }] },
  'DW-0610': { q: 'section=photos', steps: [{ dbl: 'row:ph-bridge' }, { click: '[data-action="Trash"]' }] },
  'DW-0611': { q: 'section=photos', steps: [{ dbl: 'row:ph-sunday' }] },
  'DW-0116': { q: 'section=photos&scenario=empty' },

  // ── B06: Sync history and Available offline ───────────────────────────────────────────────────────────────────
  // Sync history's frames show three problems (the sidebar's red 3, Problems · 3): the `problems` scenario, the one
  // fixture world with problems in it (M3a: the default world, DW-0401, has none).
  'DW-1301': { q: 'section=sync-history&scenario=problems', gap: 'other devices’ changes are in no command on this PC: the live list has this PC’s rows only' },
  'DW-1302': { q: 'section=sync-history&scenario=problems', steps: [{ click: '[data-filter="this-pc"]' }] },
  'DW-1303': { q: 'section=sync-history&scenario=problems', steps: [{ click: '[data-filter="other"]' }], gap: 'other devices’ changes are in no command on this PC: the filter shows none' },
  'DW-0428': { q: 'section=sync-history&scenario=problems', steps: [{ click: '[data-filter="problems"]' }] },
  'DW-1304': { q: 'section=sync-history&scenario=problems', steps: [{ click: '[data-filter="other"]' }, { fill: 'field:Search history', value: 'contract' }], gap: 'other devices’ changes are in no command on this PC' },
  'DW-1305': { q: 'section=sync-history&scenario=empty', reads: { popover_snapshot: { ...emptySnapshot() } } },
  'DW-0715': { q: 'section=available-offline' },
  'DW-1306': { q: 'section=available-offline', steps: [{ context: 'row:o-invoices' }] },
  'DW-0120': { q: 'section=available-offline&scenario=empty' },

  // ── B07: Open files ───────────────────────────────────────────────────────────────────────────────────────────
  'DW-1501': { q: 'section=open&id=o-raw' },
  'DW-1502': { q: 'section=open&id=o-raw', steps: [{ wait: 'main' }, { key: 'Alt+Shift+P' }] },
  'DW-1503': { q: 'section=open&id=o-audio' },
  'DW-1504': { q: 'section=open&id=o-audio', native: 'Quick Settings’ media controls are drawn by Windows' },
  'DW-1505': { q: 'section=open&id=o-pdf' },
  'DW-1506': { q: 'section=open&id=i-agreement', steps: [{ wait: 'main' }, { key: 'Ctrl+f' }, { fill: 'input', value: 'terrace' }] },
  'DW-1512': { q: 'section=open&id=o-py' },
  'DW-1513': { q: 'section=open&id=o-md' },
  'DW-1514': { q: 'section=open&id=o-csv' },
  'DW-1515': { q: 'section=open&id=o-zip' },
  // M3d: Studio plan.pdf's read stays in flight in the `decrypting` world; it sits in Lisbon trip, where Cancel goes.
  'DW-1516': { q: 'section=open&id=o-pdf&scenario=decrypting', steps: [{ wait: '[data-state="decrypting"]' }] },
  'DW-1517': { q: 'section=open&id=o-pdf', gap: 'Too large shows on a quota answer from readBytes; the fixtures never give one' },
  // server.log has no fixture text, so reading it fails: the live failure path.
  'DW-1518': { q: 'section=open&id=o-log' },
  // M3d: in the `newer` world Anna's iPhone saves version 4 while the note is open: the engine's item event, the banner.
  'DW-1519': { q: 'section=open&id=o-md&scenario=newer', steps: [{ wait: '[data-newer-banner]' }] },
  'DW-1520': { q: 'section=files', steps: [{ click: '[data-anchor="new"]' }] },
  // M3d: New file is opened from Files' New menu (DW-1520), so Cancel has the folder to go back to.
  'DW-1521': { q: 'section=files', steps: [{ click: '[data-anchor="new"]' }, { click: menuItem('new-file') }, { wait: '[data-type="md"]' }] },
  'DW-1522': { q: 'section=files', steps: [{ click: '[data-anchor="new"]' }, { click: menuItem('new-file') }, { click: '[data-type="txt"]' }, { click: 'btn:Continue' }] },
  'DW-1523': { q: 'section=open&id=o-md&sub=edit', steps: [{ wait: '.cm-content' }, { click: '.cm-content' }, { key: 'Ctrl+End' }, { type: '.cm-content', text: '\n3. Send the notes round' }] },
  'DW-1524': { q: 'section=open&id=o-md&sub=edit', steps: [{ wait: '.cm-content' }, { click: '.cm-content' }, { type: '.cm-content', text: '\n3. Send the notes round' }, { key: 'Ctrl+s' }] },
  // M3d: the editor's own back ("Back to the folder"); `btn:Back` matched the sidebar's Backups first.
  'DW-1525': { q: 'section=open&id=o-md&sub=edit', steps: [{ wait: '.cm-content' }, { click: '.cm-content' }, { type: '.cm-content', text: '\n3. Send the notes round' }, { click: 'btn:Back to the folder' }, { wait: 'btn:Keep editing' }] },
  // M3d: in the `edit-conflict` world Anna's iPhone saves version 4 while the note is edited: the save meets it.
  'DW-1526': {
    q: 'section=open&id=o-md&sub=edit&scenario=edit-conflict',
    steps: [{ wait: '.cm-content' }, { click: '.cm-content' }, { type: '.cm-content', text: '\n3. Send the notes round' }, { key: 'Ctrl+s' }, { wait: 'btn:Keep both' }],
  },
  'DW-1527': { q: 'section=open&id=o-log', gap: 'server.log has no fixture text (B07 draws its 41,206 lines in the frame only)' },

  // ── B08: Office ───────────────────────────────────────────────────────────────────────────────────────────────
  // M3d: opened from Files › Documents (a double-click), so Back and Alt+← have the folder behind them.
  'DW-1507': { q: 'section=files&id=f-documents', steps: [{ dbl: 'row:x-agreement' }, { wait: '[data-office]' }], gap: OFFICE_ENGINE },
  'DW-1550': { q: 'section=open&id=x-agreement&sub=office', gap: OFFICE_ENGINE },
  // M3d: Commands opens with Ctrl+K and finds "head" (the palette works without the engine; running a command needs it).
  'DW-1551': { q: 'section=open&id=x-agreement&sub=office', steps: [{ wait: 'main' }, { key: 'Ctrl+k' }, { fill: 'field:Search commands, styles, files', value: 'head' }], gap: OFFICE_ENGINE },
  'DW-1552': { q: 'section=open&id=x-agreement&sub=office', gap: OFFICE_ENGINE },
  'DW-1553': { q: 'section=open&id=x-agreement&sub=office', gap: OFFICE_ENGINE },
  'DW-1554': { q: 'section=open&id=x-agreement&sub=office', gap: OFFICE_ENGINE },
  'DW-1555': { q: 'section=open&id=x-agreement&sub=office', gap: OFFICE_ENGINE },
  'DW-1556': { q: 'section=open&id=x-agreement&sub=office' },
  'DW-1557': { q: 'section=open&id=x-agreement&sub=office', steps: [{ click: 'btn:About' }, { wait: 'btn:Third-party licenses' }], gap: OFFICE_ENGINE },
  'DW-1511': { q: 'section=open&id=x-proposal&sub=office', gap: OFFICE_ENGINE },
  'DW-1508': { q: 'section=open&id=x-budget&sub=office', gap: OFFICE_ENGINE },
  'DW-1559': { q: 'section=open&id=x-budget&sub=office', gap: OFFICE_ENGINE },
  'DW-1509': { q: 'section=open&id=x-recap&sub=office', gap: OFFICE_ENGINE },
  'DW-1560': { q: 'section=open&id=x-recap&sub=office', gap: OFFICE_ENGINE },
  'DW-1561': { q: 'section=open&id=x-recap&sub=office', gap: OFFICE_ENGINE },
  'DW-1558': { q: 'section=files', steps: [{ click: '[data-anchor="new"]' }, { click: menuItem('new-document') }] },
  // M3d: Service agreement.docx is listed in Documents now, so the menu is on the frame's own file.
  'DW-1562': { q: 'section=files&id=f-documents', steps: [{ context: 'row:x-agreement' }] },
}

/** DW-1305: the board's snapshot with nothing in it (a new account: no activity, no conflicts, nothing waiting). */
function emptySnapshot() {
  return { ...SCENARIOS['up-to-date'].snapshot, storage: null, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [] }
}
