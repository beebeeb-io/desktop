/**
 * Must-render row 10, lead ruling FT-row10: Windows has no Status page, so an engine refusal
 * (`OTHER_ACCOUNT_ON_WINDOWS`: "This PC still holds local files of another Beebeeb account…") is
 * rendered on the existing sync-status card, the main window's live status strip on Home, sentence
 * only (no action: W2's way out stays a Windows decision, 1748 / spec C). Ruling P made the refusal a
 * sentence so that it is never silent.
 *
 * The real `HomeView` declaration runs with its layout components stubbed as plain tags, so the test
 * reads what a person reads. What it does not prove: layout on a Windows machine.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'
import { rustStr } from './fixtures/rustConstants'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const SENTENCE = rustStr('account_binding.rs', 'OTHER_ACCOUNT_ON_WINDOWS')
const status = (engine_refusal: unknown) => ({ logged_in: true, engine: 'stopped', sync_root: 'C:\\Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal })

function openHome(syncStatus: unknown) {
  const m = mount('WindowsApp.tsx', 'HomeView', {
    backend: {},
    props: { status: syncStatus, usage: null, storage: null },
    bindings: {
      usePlatformName: () => 'windows',
      thisDeviceNoun: () => 'this PC',
      formatBytes: (n: number) => `${n} B`,
      accountProfile: async () => ({ ok: false, reason: 'signed out', unsupported: false }),
      accountActivity: async () => ({ ok: false, reason: 'signed out', unsupported: false }),
      homeRelativeTime: () => '',
      homeEventVisual: () => ({ icon: 'clock', dot: '' }),
      homeHumanizeType: (t: string) => t,
      T: new Proxy({}, { get: () => '' }),
      Card: 'card', Chip: 'chip', PageHeader: 'header', Skeleton: 'skeleton', NavIcon: 'icon',
    },
  })
  mounted.push(m)
  return m
}

describe('Windows: the engine refusal on the sync-status card (row 10)', () => {
  test('a refusal is its sentence on the live status strip, as a status line with no action', async () => {
    const m = openHome(status({ code: 'other_account_on_windows', sentence: SENTENCE }))
    await m.flush()
    const lines = m.elements().filter((el) => el.props['data-engine-refusal'] != null)
    expect(lines.map((el) => textOf(el.props.children))).toEqual([SENTENCE])
    expect(lines[0].props.role).toBe('status')
    expect(m.elements().filter((el) => el.type === 'button')).toEqual([])
  })

  test('no refusal, no line', async () => {
    const m = openHome(status(null))
    await m.flush()
    expect(m.elements().filter((el) => el.props['data-engine-refusal'] != null)).toEqual([])
    expect(textOf(m.tree())).not.toContain(SENTENCE)
  })
})
