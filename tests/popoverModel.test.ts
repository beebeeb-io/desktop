/**
 * The popover's model (task 1683 slice 3; two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §6.3,
 * §6.4, §10.1, §11): one snapshot in, what the popover shows out. Every §6.3 state has a fixture in
 * `tests/fixtures/popover/`; each test reads one and pins the header, the body, the status row, the one action, the
 * footer and where focus starts.
 *
 * What this proves: the words and the precedence. What it does NOT prove: the rendering (tests/popoverApp.test.tsx),
 * layout (tests/render-popover.mjs), or anything on a Mac.
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'
import { parsePopoverSnapshot } from '../src/popoverContract'
import {
  ACCOUNT_STATE_COPY,
  activityTime,
  MACOS_DOWNLOAD_URL,
  popoverView,
  type PopoverInput,
  type PopoverScreen,
  type PopoverView,
  UPDATE_DOWNLOAD_HINT,
} from '../src/popoverModel'

const DIR = new URL('./fixtures/popover/', import.meta.url)
const fixture = (name: string): PopoverInput => JSON.parse(readFileSync(new URL(`${name}.json`, DIR), 'utf8'))
const NOW = fixture('synced').generated_at
/** Fixed locale and UTC days, so "84,3 GB" and "Yesterday" do not depend on the machine. */
const view = (input: PopoverInput, now = NOW): PopoverScreen => popoverView(input, now, { locale: 'nl', tzOffsetMinutes: 0 })
const state = (input: PopoverInput, now = NOW): PopoverView => {
  const screen = view(input, now)
  if (screen.kind !== 'state') throw new Error(`expected a popover, got ${screen.kind}`)
  return screen
}
const message = (v: PopoverView) => {
  if (v.body.kind !== 'message') throw new Error(`expected a message body, got ${v.body.kind}`)
  return v.body
}
const ALL_ENABLED = { open_location: true, view_online: true, add_storage: true }

describe('the fixtures stay on the contract', () => {
  test('every fixture is a snapshot the validator accepts', () => {
    const names = readdirSync(DIR).filter((f) => f.endsWith('.json'))
    expect(names.length).toBe(17)
    for (const name of names) {
      const raw = JSON.parse(readFileSync(new URL(name, DIR), 'utf8'))
      expect({ name, ok: parsePopoverSnapshot(raw) !== null }).toEqual({ name, ok: true })
    }
  })
})

describe('§6.2: no session, no popover', () => {
  test('signed out and session ended open the window instead', () => {
    expect(view(fixture('signed-out'))).toEqual({ kind: 'no_session' })
    expect(view(fixture('session-ended'))).toEqual({ kind: 'no_session' })
  })

  test('no session wins over a send phase', () => {
    expect(view({ ...fixture('session-ended'), sign_out_sending: { changes_left: 2 } })).toEqual({ kind: 'no_session' })
  })
})

describe('§6.3 states, one fixture each', () => {
  test('up to date: email, storage, Recent activity with 5 rows, ✓ Up to date and Encrypted, focus on the gear', () => {
    const v = state(fixture('synced'))
    expect(v.header).toEqual({ email: 'sam@example.eu', initial: 'S', storage: { label: 'Using 84,3 GB of 200 GB', percent: expect.closeTo(42.15, 6), red: false } })
    expect(v.body.kind).toBe('activity')
    if (v.body.kind !== 'activity') return
    expect(v.body.label).toBe('Recent activity')
    expect(v.body.rows.map((r) => [r.name, r.meta])).toEqual([
      ['Q3 ledger reconciliation.xlsx', 'Investigations · 2 min ago'],
      ['Source documents, batch 12.pdf', 'Ledger gap · 14 min ago'],
      ['IMG_4821.HEIC', 'Photos · 1 hour ago'],
      ['Board notes, 29 Sept.md', 'Team shared · Yesterday'],
      ['Interview 07, transcript.docx', 'Investigations · Yesterday'],
    ])
    expect(v.status).toEqual({ kind: 'ok', label: 'Up to date', encrypted: true })
    expect(v.update).toBeNull()
    expect(v.attention).toBeNull()
    expect(v.footer).toEqual(ALL_ENABLED)
    expect(v.focus).toBe('gear')
  })

  test('syncing: files left with size and percent, rows with progress, then Done', () => {
    const v = state(fixture('syncing'))
    if (v.body.kind !== 'syncing') throw new Error(v.body.kind)
    expect(v.body.left).toBe('14 files left')
    expect(v.body.total).toBe('1,2 GB · 38 %')
    expect(v.body.percent).toBe(38)
    expect(v.body.active.map((r) => [r.meta, r.progress])).toEqual([
      ['Investigations · Encrypting and uploading · 62 %', { percent: 62, encrypting: true, valueText: '62 percent, encrypting and uploading' }],
      ['Photos · Encrypting and uploading · 21 %', { percent: 21, encrypting: true, valueText: '21 percent, encrypting and uploading' }],
      ['Team shared · Downloading · 88 %', { percent: 88, encrypting: false, valueText: '88 percent, downloading' }],
    ])
    expect(v.body.doneLabel).toBe('Done')
    expect(v.body.done.map((r) => r.name)).toEqual(['Q3 ledger reconciliation.xlsx'])
    expect(v.status).toEqual({ kind: 'busy', label: 'Syncing', encrypted: true })
    expect(v.footer).toEqual(ALL_ENABLED)
  })

  test('needs a decision: the attention row with Review…, then the conflicting files and activity; focus on Review…', () => {
    const v = state(fixture('needs-decision'))
    expect(v.attention).toEqual({
      title: '2 files need a decision',
      text: 'Both changed on two devices.',
      action: { label: 'Review…', command: 'open_review', primary: false },
    })
    if (v.body.kind !== 'activity') throw new Error(v.body.kind)
    expect(v.body.rows.map((r) => [r.name, r.meta])).toEqual([
      ['Board notes, 29 Sept.md', 'Team shared · Changed on two devices'],
      ['Budget 2027.xlsx', 'Changed on two devices'],
      ['Q3 ledger reconciliation.xlsx', 'Investigations · 2 min ago'],
    ])
    expect(v.status).toEqual({ kind: 'ok', label: 'Up to date', encrypted: true })
    expect(v.focus).toBe('open_review')
  })

  test('update available: the version and, on a Mac, Download update (task 2176), over the state below', () => {
    const v = state(fixture('update-available'))
    expect(v.update).toEqual({
      line: 'Version 0.8.14 is available.',
      hint: UPDATE_DOWNLOAD_HINT,
      action: { label: 'Download update', command: 'download_update', primary: true },
    })
    expect(UPDATE_DOWNLOAD_HINT).toBe('Drag Beebeeb into Applications to replace this version.')
    expect(MACOS_DOWNLOAD_URL).toBe('https://beebeeb.io/download/macos-dmg')
    expect(v.body.kind).toBe('activity')
    expect(v.status).toEqual({ kind: 'ok', label: 'Up to date', encrypted: true })
    expect(v.focus).toBe('download_update')
  })

  test.each(['beta', 'alpha', undefined])('a %s update offers its release, never the stable disk image', (channel) => {
    const input = fixture('update-available')
    input.update.channel = channel
    const v = state(input)
    expect(v.update?.action.label).toBe('View release')
    expect(v.update?.hint).toContain('release page')
  })

  test('paused: the waiting changes and Resume sync; no status row', () => {
    const v = state(fixture('paused'))
    expect(message(v)).toEqual({
      kind: 'message', tone: 'neutral', icon: 'pause', title: 'Sync is paused',
      lines: ['3 changes on this Mac are waiting. Nothing syncs until you resume.'], reason: null,
      action: { label: 'Resume sync', command: 'resume', primary: true },
    })
    expect(v.status).toBeNull()
    expect(v.focus).toBe('resume')
  })

  test('offline: Can’t reach Beebeeb with Try again; the two web actions are disabled', () => {
    const v = state(fixture('offline'))
    expect(message(v)).toEqual({
      kind: 'message', tone: 'neutral', icon: 'globe', title: 'Can’t reach Beebeeb',
      lines: ['Check your connection. Files on this Mac are unchanged.'], reason: null,
      action: { label: 'Try again', command: 'retry_sync', primary: true },
    })
    expect(v.footer).toEqual({ open_location: true, view_online: false, add_storage: false })
  })

  test('error: Sync stopped, the mono reason (≤ 30 characters) and Try again', () => {
    const v = state(fixture('error'))
    expect(message(v)).toEqual({
      kind: 'message', tone: 'error', icon: 'alert', title: 'Sync stopped',
      lines: ['Beebeeb couldn’t finish syncing. Files on this Mac are unchanged.'], reason: 'reason: server 503',
      action: { label: 'Try again', command: 'retry_sync', primary: true },
    })
    const long = state({ ...fixture('error'), reason: { code: 'a_reason_code_far_longer_than_thirty', detail: null } })
    expect(message(long).reason!.length).toBe(30)
  })

  test('vault locked: email only, Unlock vault', () => {
    const v = state(fixture('locked'))
    expect(v.header).toEqual({ email: 'sam@example.eu', initial: 'S', storage: null })
    expect(message(v)).toEqual({
      kind: 'message', tone: 'encryption', icon: 'lock', title: 'Your vault is locked',
      lines: ['Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.'], reason: null,
      action: { label: 'Unlock vault', command: 'unlock', primary: true },
    })
    expect(v.focus).toBe('unlock')
  })

  test('storage full: the red bar, the waiting changes and Add storage', () => {
    const v = state(fixture('storage-full'))
    expect(v.header.storage).toEqual({ label: 'Using 200 GB of 200 GB', percent: 100, red: true })
    expect(message(v)).toEqual({
      kind: 'message', tone: 'error', icon: 'db', title: 'Your storage is full',
      lines: ['3 changes can’t upload until you free up space or add storage. They stay on this Mac.'], reason: null,
      action: { label: 'Add storage', command: 'add_storage', primary: true },
    })
  })

  test('Finder adding: activity, and the status row says so; Open in Finder disabled', () => {
    const v = state(fixture('finder-adding'))
    expect(v.body.kind).toBe('activity')
    expect(v.status).toEqual({ kind: 'busy', label: 'Adding Beebeeb to Finder…', encrypted: false })
    expect(v.footer).toEqual({ open_location: false, view_online: true, add_storage: true })
  })

  test('Finder failed: spec A’s sentence and action per reason, the mono reason; Open in Finder disabled', () => {
    const timeout = state(fixture('finder-failed'))
    expect(message(timeout)).toEqual({
      kind: 'message', tone: 'error', icon: 'alert', title: 'Couldn’t add Beebeeb to Finder',
      lines: ['macOS didn’t finish adding Beebeeb to Finder.'], reason: 'reason: timeout',
      action: { label: 'Try again', command: 'finder_try_again', primary: true },
    })
    expect(timeout.footer.open_location).toBe(false)
    const dmg = message(state(fixture('finder-not-in-applications')))
    expect([dmg.lines, dmg.action]).toEqual([
      ['Beebeeb is running from the disk image. Move it to Applications, then open it again.'],
      { label: 'Show in Finder', command: 'finder_show_in_finder', primary: true },
    ])
    const off = message(state(fixture('finder-user-disabled')))
    expect([off.title, off.lines, off.action]).toEqual([
      'Couldn’t add Beebeeb to Finder',
      ['Beebeeb is turned off in System Settings.'],
      { label: 'Open System Settings', command: 'finder_open_system_settings', primary: true },
    ])
  })

  test('Finder missing claims nothing (spec A, "Where Missing rests"): the sync part shows; Open in Finder disabled', () => {
    const v = state(fixture('finder-missing'))
    expect(v.body.kind).toBe('activity')
    expect(v.status).toEqual({ kind: 'ok', label: 'Up to date', encrypted: true })
    expect(v.footer.open_location).toBe(false)
    const paused = state({ ...fixture('finder-missing'), sync_phase: 'paused', paused: true, pending_changes: 1 })
    expect(message(paused).lines).toEqual(['1 change on this Mac is waiting. Nothing syncs until you resume.'])
  })

  test('sign-out in progress: 1887’s P0–P2 word for word, and only Open Beebeeb (neutral)', () => {
    const v = state(fixture('sign-out-sending'))
    expect(message(v)).toEqual({
      kind: 'message', tone: 'busy', icon: null, title: 'Sending changes first',
      lines: ['Sending 3 changes before signing out…', 'This takes at most a minute.'], reason: null,
      action: { label: 'Open Beebeeb', command: 'open_beebeeb', primary: false },
    })
    expect(v.focus).toBe('open_beebeeb')
    const one = state({ ...fixture('sign-out-sending'), sign_out_sending: { changes_left: 1 } })
    expect(message(one).lines).toEqual(['Sending 1 change before signing out…', 'This takes at most a minute.'])
    const none = state({ ...fixture('sign-out-sending'), sign_out_sending: { changes_left: 0 } })
    expect(message(none).lines).toEqual(['This takes at most a minute.'])
  })
})

describe('precedence', () => {
  test('a send phase is shown over every other state', () => {
    for (const name of ['locked', 'paused', 'finder-failed', 'storage-full', 'syncing']) {
      expect([name, message(state({ ...fixture(name), sign_out_sending: { changes_left: 2 } })).title]).toEqual([name, 'Sending changes first'])
    }
  })

  test('the update row and the attention row show over a list state, never over a message state', () => {
    const over = { update: { version: '0.8.14' }, conflicts: fixture('needs-decision').conflicts }
    for (const name of ['synced', 'syncing', 'finder-adding', 'finder-missing']) {
      const v = state({ ...fixture(name), ...over })
      expect([name, v.update?.line, v.attention?.title]).toEqual([name, 'Version 0.8.14 is available.', '2 files need a decision'])
    }
    for (const name of ['paused', 'offline', 'error', 'locked', 'storage-full', 'finder-failed', 'sign-out-sending']) {
      const v = state({ ...fixture(name), ...over })
      expect([name, v.update, v.attention]).toEqual([name, null, null])
    }
  })

  test('both rows: the update row first, focus on its action, and fewer activity rows', () => {
    const v = state({ ...fixture('synced'), update: { version: '0.8.14' }, conflicts: fixture('needs-decision').conflicts })
    expect(v.focus).toBe('download_update')
    if (v.body.kind !== 'activity') throw new Error(v.body.kind)
    expect(v.body.rows.length).toBe(1)
  })

  test('the activity list keeps to the fixed body: 5 rows, 3 under one row on top', () => {
    const rows = (input: PopoverInput) => { const b = state(input).body; return b.kind === 'activity' ? b.rows.length : -1 }
    expect(rows(fixture('synced'))).toBe(5)
    expect(rows(fixture('update-available'))).toBe(3)
    expect(rows(fixture('needs-decision'))).toBe(3)
  })

  test('a conflicting file is listed once: as the conflict, not again as activity', () => {
    const input = fixture('synced')
    const v = state({ ...input, conflicts: { count: 1, files: [{ file_id: 'f1', file_name: 'Q3 ledger reconciliation.xlsx' }] } })
    if (v.body.kind !== 'activity') throw new Error(v.body.kind)
    expect(v.body.rows.map((r) => [r.name, r.meta])).toEqual([
      ['Q3 ledger reconciliation.xlsx', 'Investigations · Changed on two devices'],
      ['Source documents, batch 12.pdf', 'Ledger gap · 14 min ago'],
      ['IMG_4821.HEIC', 'Photos · 1 hour ago'],
    ])
  })

  test('syncing shows at most 4 rows: moving files first, then Done', () => {
    const many = fixture('syncing')
    const moving = Array.from({ length: 6 }, (_, i) => ({ ...many.activity[0], id: `m${i}` }))
    const v = state({ ...many, activity: [...moving, ...fixture('synced').activity] })
    if (v.body.kind !== 'syncing') throw new Error(v.body.kind)
    expect([v.body.active.length, v.body.doneLabel, v.body.done.length]).toEqual([4, null, 0])
  })
})

describe('words', () => {
  test('one of each: singular forms', () => {
    const syncing = state({ ...fixture('syncing'), engine: { ...fixture('syncing').engine, files_remaining: 1 } })
    expect(syncing.body.kind === 'syncing' && syncing.body.left).toBe('1 file left')
    const conflict = state({ ...fixture('synced'), conflicts: { count: 1, files: [{ file_id: 'x', file_name: 'a.md' }] } })
    expect(conflict.attention?.title).toBe('1 file needs a decision')
    expect(message(state({ ...fixture('storage-full'), pending_changes: 1 })).lines).toEqual([
      '1 change can’t upload until you free up space or add storage. They stay on this Mac.',
    ])
    expect(message(state({ ...fixture('paused'), pending_changes: 0 })).lines).toEqual(['Nothing syncs until you resume.'])
  })

  test('an unknown progress shows no percentage, never a guess', () => {
    const input = fixture('syncing')
    input.activity[0] = { ...input.activity[0], done_bytes: null, total_bytes: null }
    const v = state({ ...input, engine: { ...input.engine, bytes_total: null, bytes_done: null } })
    if (v.body.kind !== 'syncing') throw new Error(v.body.kind)
    expect(v.body.total).toBeNull()
    expect(v.body.active[0].meta).toBe('Investigations · Encrypting and uploading')
    expect(v.body.active[0].progress).toEqual({ percent: null, encrypting: true, valueText: 'encrypting and uploading' })
  })

  test('a failed row says so; a file at the vault root has no folder', () => {
    const input = fixture('synced')
    const failed = { ...input.activity[0], id: 'e1', state: 'failed' as const, direction: null }
    const root = { ...input.activity[1], id: 'r1', folder: '' }
    const v = state({ ...input, activity: [failed, root] })
    if (v.body.kind !== 'activity') throw new Error(v.body.kind)
    expect(v.body.rows.map((r) => [r.meta, r.failed, r.direction])).toEqual([
      ['Investigations · Couldn’t sync', true, null],
      ['14 min ago', false, 'up'],
    ])
  })

  test('VoiceOver reads each row as one sentence (§11)', () => {
    const v = state(fixture('synced'))
    if (v.body.kind !== 'activity') throw new Error(v.body.kind)
    expect(v.body.rows[0].label).toBe('Q3 ledger reconciliation.xlsx, uploaded, Investigations, 2 minutes ago')
    expect(v.body.rows[2].label).toBe('IMG_4821.HEIC, downloaded, Photos, 1 hour ago')
    expect(v.body.rows[2].icon).toBe('image')
    const s = state(fixture('syncing'))
    if (s.body.kind !== 'syncing') throw new Error(s.body.kind)
    expect(s.body.active[0].label).toBe('Interview 09, raw audio.wav, encrypting and uploading, Investigations, 62 percent')
  })

  test('activity times', () => {
    const t = (ago: number) => activityTime(NOW - ago, NOW, 0)
    expect([t(10), t(59), t(60), t(59 * 60), t(3600), t(2 * 3600), t(26 * 3600), t(3 * 86400)]).toEqual([
      { short: 'Just now', long: 'just now' },
      { short: 'Just now', long: 'just now' },
      { short: '1 min ago', long: '1 minute ago' },
      { short: '59 min ago', long: '59 minutes ago' },
      { short: '1 hour ago', long: '1 hour ago' },
      { short: '2 hours ago', long: '2 hours ago' },
      { short: 'Yesterday', long: 'yesterday' },
      { short: '22 Oct', long: '22 October' },
    ])
    expect(activityTime(NOW + 30, NOW, 0).short).toBe('Just now')
  })

  test('nothing a person reads in the popover uses a developer or Windows word (§10.1)', () => {
    const banned = /\b(daemon|hydrat\w*|engine|control cent(er|re)|file surface|namespace|state db|socket|telemetry|heartbeat|wired|this build|PC|Explorer|File Explorer|Windows|OneDrive)\b/i
    const strings = (value: unknown): string[] =>
      typeof value === 'string' ? [value] : Array.isArray(value) ? value.flatMap(strings) : value && typeof value === 'object' ? Object.values(value).flatMap(strings) : []
    const names = readdirSync(DIR).filter((f) => f.endsWith('.json')).map((f) => f.replace(/\.json$/, ''))
    const read = names.flatMap((n) => strings(view(fixture(n)))).concat(strings(ACCOUNT_STATE_COPY))
    expect(read.length).toBeGreaterThan(100)
    expect(read.filter((s) => banned.test(s))).toEqual([])
    expect(read.filter((s) => /\.\.\.|'/.test(s))).toEqual([])
  })
})

describe('not built in this slice: spec C’s account states wait for task 1747 (copy pinned here)', () => {
  test('checking, no plan and update required keep spec C’s words', () => {
    expect(ACCOUNT_STATE_COPY).toEqual({
      checking: { title: null, lines: [] },
      no_plan: {
        title: 'Choose a plan',
        lines: ['Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.'],
        waiting: 'Waiting for your plan.',
        action: 'Choose a plan on beebeeb.io',
      },
      update_required: {
        title: 'Update Beebeeb',
        lines: ['This version of Beebeeb is too old to connect. Update to continue.'],
        action: 'Update now',
      },
    })
  })
})
