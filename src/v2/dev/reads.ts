// Dev only (batch M2), never bundled: a plain browser has no Rust, so the v2 dev server answers READ commands with the
// board's fixture data, the same modules the frame registry renders. Writes are never answered here: in a browser they
// stay "unavailable", which the map audit reports as command-unsupported. A live state's recipe may add or replace
// reads (`reads` in ./states-*.ts), e.g. a signed-out `sync_status` for the sign-in window.
import type { CommandResult } from '../../desktopApi'
import type { ActivityRow, PopoverSnapshot } from '../../popoverContract'
import { BY_ME_PEOPLE, BY_ME_LINKS, INVITATIONS, LISBON_PEOPLE, REQUESTS, SAFETY_NUMBER } from '../source/fixtures/shared'
import { HISTORY } from '../source/fixtures/photos'
import { FIXTURE_ACCOUNT } from '../source/fixtures/account'
import { SCENARIOS } from '../tray/fixtures'
import { INSIGHTS_BREAKDOWN } from '../source/fixtures/settings-today'

/** A recipe's read: a value, `{ error }` for a refusal (`unsupported` for "this build lacks it"), or `{ pending: true }` for
 *  a host that has not answered yet (a busy state: the call never settles). */
export type Read = unknown | { error: string; unsupported?: boolean }
export type Reads = Record<string, Read | ((args?: Record<string, unknown>) => Read)>

const at = (iso: string) => Math.floor(new Date(iso).getTime() / 1000)

/** B06's Sync history rows that happened on this PC, as the flyout's activity (what the live section reads). */
function historyActivity(): ActivityRow[] {
  return HISTORY.filter((r) => r.device === 'This PC' && !r.trashed).map((r) => {
    const failed = !!r.problem
    const down = /^Downloaded from /.test(r.what)
    const folder = r.what.replace(/^(Didn’t sync · |Uploaded to |Downloaded from |Moved to |Backed up to )/, '')
    // Rust's activity row id is the file's id (popover_data/snapshot.rs), so a row Sync history opens names its item.
    return {
      id: r.itemId ?? r.id, name: r.name, folder, path: `${folder}/${r.name}`, direction: failed ? null : down ? 'down' : 'up',
      state: failed ? 'failed' : 'done', at: at(r.at), size_bytes: null, done_bytes: null, total_bytes: null,
    }
  })
}

/** The main window's snapshot (Sync history) and the flyout's (DW-1202: up to date). */
export function boardSnapshot(): PopoverSnapshot {
  return { ...SCENARIOS['up-to-date'].snapshot, activity: historyActivity() }
}

export const BOARD_READS: Reads = {
  // The gate moved to Rust (task 2445) and fails closed when the host cannot answer: a browser state is signed in
  // unless its recipe says otherwise (states-windows.ts: signed_out, locked, no_plan).
  auth_gate_state: 'ready',
  share_list: { invitations: INVITATIONS, links: BY_ME_LINKS, requests: REQUESTS, pending: BY_ME_PEOPLE.filter((p) => p.pending).map((p) => p.id) },
  share_people: LISBON_PEOPLE,
  share_safety_number: { groups: SAFETY_NUMBER },
  account_profile: { email: FIXTURE_ACCOUNT.email, email_verified: FIXTURE_ACCOUNT.email_verified, totp_enabled: true },
  popover_snapshot: (args?: Record<string, unknown>) => (args?.activityLimit === 200 ? boardSnapshot() : SCENARIOS['up-to-date'].snapshot),
  // Storage › Storage insights (DW-0783): the board's breakdown, the 50 largest asked for.
  account_storage_breakdown: INSIGHTS_BREAKDOWN,
}

/**
 * The answer function the fixture host (./host.ts) calls: the recipe's reads over the board's, then its one-shot answers
 * (`answers`: each answers the next call once, in order), the host's reply a step needs to move the flow into the
 * frame's state, e.g. desktop_login asking for the second factor. Once used, that command is unanswered again, so the
 * map audit's own action on the state meets the browser as it is (command-unsupported), never a canned success.
 */
export function answerFrom(extra: Reads = {}, once: Record<string, Read[]> = {}): (name: string, args?: Record<string, unknown>) => CommandResult<unknown> | Promise<CommandResult<unknown>> | undefined {
  const table: Reads = { ...BOARD_READS, ...extra }
  const queue = new Map(Object.entries(once).map(([k, v]) => [k, [...v]]))
  const result = (value: Read): CommandResult<unknown> | Promise<CommandResult<unknown>> => {
    if (value && typeof value === 'object' && (value as { pending?: boolean }).pending === true) return new Promise(() => {})
    if (value && typeof value === 'object' && 'error' in (value as object) && Object.keys(value as object).every((k) => k === 'error' || k === 'unsupported')) {
      const e = value as { error: string; unsupported?: boolean }
      return { ok: false, reason: e.error, unsupported: !!e.unsupported }
    }
    return { ok: true, value }
  }
  return (name, args) => {
    const next = queue.get(name)
    if (next?.length) return result(next.shift())
    if (!(name in table)) return undefined
    const entry = table[name]
    return result(typeof entry === 'function' ? (entry as (a?: Record<string, unknown>) => Read)(args) : entry)
  }
}
