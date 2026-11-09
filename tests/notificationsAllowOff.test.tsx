// Settings › Notifications: Allow off hides every group under the first one, and each switch is a value in the desktop
// config, not component state that a reopen throws away. The harness runs the real pane and the real useConfig against
// a scripted get_desktop_config / set_desktop_config. It does not prove a toast is delivered.
import { afterEach, describe, expect, test } from 'bun:test'
import { command, DEFAULT_CONFIG } from '../src/desktopApi'
import { actionFailure } from '../src/v2/sections/files/model'
import { Btn, Group, Page, Row, Switch } from '../src/v2/sections/settings/shell/kit'
import { board } from '../src/v2/board'
import { openExternal } from '../src/v2/external'
import { hasTauri } from '../src/v2/source'
import { mount, textOf, type Mounted, type TreeNode } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) {
    const m = mounted.pop()!
    m.unmount()
    m.close()
  }
})

const GROUPS = ['Security', 'Your account', 'Sync on this PC', 'Backups on this PC']
const SHOWN = [
  'Allow notifications',
  'Shared with you',
  'File updates',
  'Storage warnings',
  'Sync stopped',
  'Files need a decision',
  'Sync finished',
  'Space on this PC',
  'Backup finished',
  'Backup needs attention',
]

function walk(node: unknown): TreeNode[] {
  if (Array.isArray(node)) return node.flatMap(walk)
  if (!node || typeof node !== 'object' || !('props' in node)) return []
  const el = node as TreeNode
  return [el, ...Object.values(el.props).flatMap(walk)]
}

function ofType(m: Mounted, type: unknown): TreeNode[] {
  return walk(m.tree()).filter((el) => el.type === type)
}

function titles(m: Mounted): string[] {
  return ofType(m, Group).map((el) => el.props.title).filter((title) => typeof title === 'string')
}

function rows(m: Mounted): string[] {
  return ofType(m, Row).map((el) => el.props.label).filter((label) => typeof label === 'string')
}

function switchNamed(m: Mounted, label: string): TreeNode {
  const found = ofType(m, Switch).filter((el) => el.props.label === label)
  if (found.length !== 1) throw new Error(`${label}: expected 1 switch, found ${found.length}`)
  return found[0]!
}

async function flip(m: Mounted, label: string, next: boolean) {
  switchNamed(m, label).props.onChange(next)
  await m.flush()
}

function session(initial: Record<string, unknown> = {}) {
  const disk: Record<string, unknown> = { ...DEFAULT_CONFIG, ...initial }
  const writes: Record<string, unknown>[] = []
  const backend = {
    get_desktop_config: () => ({ ...disk }),
    set_desktop_config: ({ config }: { config: Record<string, unknown> }) => {
      writes.push({ ...config })
      Object.assign(disk, config)
      return null
    },
    account_notifications: () => ({ notifications: [], unread_count: 0 }),
  }
  const mountPane = (props: { offInWindows?: boolean } = {}) => {
    const m = mount('v2/sections/settings/shell/notifications.tsx', 'Notifications', {
      backend,
      props,
      honestRender: true,
      hookModules: [
        { file: 'v2/sections/settings/shell/data.ts', name: 'useConfig', bindings: { command, DEFAULT_CONFIG, hasTauri, board } },
        { file: 'v2/sections/settings/shell/data.ts', name: 'useRead', bindings: { command } },
      ],
      bindings: { Btn, Group, Page, Row, Switch, openExternal, actionFailure },
    })
    mounted.push(m)
    return m
  }
  return { disk, writes, mountPane }
}

function close(m: Mounted) {
  const at = mounted.indexOf(m)
  if (at >= 0) mounted.splice(at, 1)
  m.unmount()
  m.close()
}

describe('Notifications: allow off hides the rest', () => {
  test('allow off hides Security, Your account, Sync and Backups', async () => {
    const s = session()
    const m = s.mountPane()
    await m.flush()
    expect(titles(m)).toEqual(GROUPS)
    expect(ofType(m, Switch).map((el) => el.props.label)).toEqual(SHOWN)
    await flip(m, 'Allow notifications', false)
    expect(titles(m)).toEqual([])
    expect(rows(m)).toEqual(['Allow notifications', 'Recent notifications'])
    expect(ofType(m, Switch)).toHaveLength(1)
    expect(s.writes).toEqual([{ notify_allow: false }])

    const os = session().mountPane({ offInWindows: true })
    await os.flush()
    expect(titles(os)).toEqual([])
    expect(rows(os)).toEqual(['Notifications are off', 'Recent notifications'])
    expect(ofType(os, Switch)).toHaveLength(0)
    expect(ofType(os, Btn).map((el) => textOf(el).trim())).toEqual(['Open Windows Settings'])
  })

  test('allow on shows those groups with the values left there', async () => {
    const s = session({ notify_allow: false, notify_conflicts: false, notify_shared: false, notify_sync_complete: true })
    const m = s.mountPane()
    await m.flush()
    expect(titles(m)).toEqual([])
    expect(switchNamed(m, 'Allow notifications').props.on).toBe(false)
    await flip(m, 'Allow notifications', true)
    expect(titles(m)).toEqual(GROUPS)
    expect(ofType(m, Switch).map((el) => el.props.label)).toEqual(SHOWN)
    expect(switchNamed(m, 'File updates').props.on).toBe(false)
    expect(switchNamed(m, 'Shared with you').props.on).toBe(false)
    expect(switchNamed(m, 'Sync finished').props.on).toBe(true)
    expect(switchNamed(m, 'Storage warnings').props.on).toBe(true)
    expect(s.writes).toEqual([{ notify_allow: true }])
  })

  test('notify_conflicts, notify_quota_warnings and notify_sync_complete each round-trip', async () => {
    const s = session()
    const m = s.mountPane()
    await m.flush()
    expect(switchNamed(m, 'File updates').props.on).toBe(true)
    expect(switchNamed(m, 'Storage warnings').props.on).toBe(true)
    expect(switchNamed(m, 'Sync finished').props.on).toBe(false)
    await flip(m, 'File updates', false)
    await flip(m, 'Storage warnings', false)
    await flip(m, 'Sync finished', true)
    expect(s.writes).toEqual([
      { notify_conflicts: false },
      { notify_quota_warnings: false },
      { notify_sync_complete: true },
    ])
    expect(switchNamed(m, 'File updates').props.on).toBe(false)
    expect(switchNamed(m, 'Storage warnings').props.on).toBe(false)
    expect(switchNamed(m, 'Sync finished').props.on).toBe(true)
  })

  test('allow and the other switches each round-trip through config', async () => {
    const s = session()
    const m = s.mountPane()
    await m.flush()
    const cases: Array<[string, string]> = [
      ['Shared with you', 'notify_shared'],
      ['Sync stopped', 'notify_sync_stopped'],
      ['Files need a decision', 'notify_needs_decision'],
      ['Space on this PC', 'notify_local_space'],
      ['Backup finished', 'notify_backup_finished'],
      ['Backup needs attention', 'notify_backup_attention'],
      ['Allow notifications', 'notify_allow'],
    ]
    for (const [label] of cases) {
      expect(switchNamed(m, label).props.on).toBe(true)
      await flip(m, label, false)
    }
    expect(s.writes).toEqual(cases.map(([, key]) => ({ [key]: false })))
    expect(switchNamed(m, 'Allow notifications').props.on).toBe(false)
    expect(titles(m)).toEqual([])
  })

  test('a restart reads the saved switches back', async () => {
    const s = session()
    const first = s.mountPane()
    await first.flush()
    await flip(first, 'File updates', false)
    await flip(first, 'Storage warnings', false)
    await flip(first, 'Sync finished', true)
    await flip(first, 'Shared with you', false)
    await flip(first, 'Sync stopped', false)
    await flip(first, 'Files need a decision', false)
    await flip(first, 'Space on this PC', false)
    await flip(first, 'Backup finished', false)
    await flip(first, 'Backup needs attention', false)
    await flip(first, 'Allow notifications', false)
    expect(s.writes).toHaveLength(10)
    close(first)

    const second = s.mountPane()
    await second.flush()
    expect(switchNamed(second, 'Allow notifications').props.on).toBe(false)
    expect(titles(second)).toEqual([])
    expect(rows(second)).toEqual(['Allow notifications', 'Recent notifications'])
    await flip(second, 'Allow notifications', true)
    expect(titles(second)).toEqual(GROUPS)
    expect(switchNamed(second, 'File updates').props.on).toBe(false)
    expect(switchNamed(second, 'Storage warnings').props.on).toBe(false)
    expect(switchNamed(second, 'Sync finished').props.on).toBe(true)
    expect(switchNamed(second, 'Shared with you').props.on).toBe(false)
    expect(switchNamed(second, 'Sync stopped').props.on).toBe(false)
    expect(switchNamed(second, 'Files need a decision').props.on).toBe(false)
    expect(switchNamed(second, 'Space on this PC').props.on).toBe(false)
    expect(switchNamed(second, 'Backup finished').props.on).toBe(false)
    expect(switchNamed(second, 'Backup needs attention').props.on).toBe(false)
    expect(s.disk.notify_conflicts).toBe(false)
    expect(s.disk.notify_quota_warnings).toBe(false)
    expect(s.disk.notify_sync_complete).toBe(true)
    expect(s.disk.notify_allow).toBe(true)
  })
})
