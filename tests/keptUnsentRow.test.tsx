/**
 * Spec 2026-10-10 §9, F5 (task 1887): Settings › Sync shows the folder a sign-out copied unsent changes into, as its
 * own row above 1882's (macOS's folder), with "Show in Finder" and "Dismiss". The Sync tab is mounted the way
 * tests/macSettingsTabs.test.tsx mounts it: the real `useFinderSetup` hook on a scripted event bus, every other name
 * the tab reads bound to the real module.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as accountSession from '../src/accountSession'
import * as desktopApi from '../src/desktopApi'
import * as diagnosticsCopy from '../src/diagnosticsCopy'
import * as finderSetup from '../src/finderSetup'
import * as finderSetupCopy from '../src/finderSetupCopy'
import * as model from '../src/macSettingsModel'
import { KEPT_FOLDER_ROW_SENTENCE } from '../src/macSettingsModel'
import * as parts from '../src/macSettingsParts'
import * as copy from '../src/signOutCopy'
import { expand, loadComponent, mount, textOf, type Mounted, type TreeNode } from './fixtures/componentHarness'
import { KeptFolderNotes, useKeptFoldersModule } from './fixtures/keptFolderHarness'

/** The in-window dialog, reduced to what a user sees (as tests/macSettingsTabs.test.tsx draws it). */
function Modal(props: any) {
  return props.open
    ? createElement('div', { role: 'dialog', 'aria-label': props.title }, createElement('h2', null, props.title), props.children, props.footer)
    : null
}

const ConfirmSheet = loadComponent('MacSettings.tsx', 'ConfirmSheet', { React: { createElement, Fragment }, Btn: parts.Btn, Modal })
const withPinned = loadComponent('MacSettings.tsx', 'withPinned', {})

const baseBindings = {
  React: { createElement, Fragment },
  ...accountSession,
  ...desktopApi,
  ...diagnosticsCopy,
  ...model,
  ...parts,
  ...copy,
  Modal,
  ConfirmSheet,
  withPinned,
  KeptFolderNotes,
  finderActionButtonLabel: finderSetupCopy.finderActionButtonLabel,
  // The real one: with no event bus here it reports ready at once, so the rows read on mount.
  subscribeKeptFolderChanged: finderSetup.subscribeKeptFolderChanged,
}

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

/** A scripted `subscribeFinderSetup`: the registration lands on the next microtask, as the real one does. */
function finderBus() {
  const listeners: Array<(view: unknown) => void> = []
  return {
    subscribeFinderSetup(onView: (view: unknown) => void, options: { onSubscribed?: () => void } = {}) {
      listeners.push(onView)
      void Promise.resolve().then(() => options.onSubscribed?.())
      return () => { const at = listeners.indexOf(onView); if (at >= 0) listeners.splice(at, 1) }
    },
  }
}

/** The REAL `useFinderSetup` declaration, run inside the mounted Sync tab. */
const useFinderSetupModule = () => ({
  file: 'finderSetup.ts',
  name: 'useFinderSetup',
  bindings: {
    loadFinderSetup: finderSetup.loadFinderSetup,
    runFinderSetupAction: (action: finderSetupCopy.FinderSetupAction) => finderSetup.runFinderSetupAction(action, { writeClipboard: async () => {} }),
    finderSetupLoadPresentation: finderSetupCopy.finderSetupLoadPresentation,
    FINDER_ACTION_FAILED: finderSetupCopy.FINDER_ACTION_FAILED,
    FINDER_COPIED_MS: finderSetupCopy.FINDER_COPIED_MS,
    FINDER_ACTION_COMMAND: finderSetup.FINDER_ACTION_COMMAND,
    commandUnavailableLabel: desktopApi.commandUnavailableLabel,
    subscribeFinderSetup: finderBus().subscribeFinderSetup,
    loadEngineRefusal: desktopApi.loadEngineRefusal,
  },
})

const settings = { state: { status: 'ready', config: { upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'dark', local_cache_limit_bytes: 0 } }, save: async () => {}, reload: async () => {} }

/** The Sync tab's backend: Finder added, two folders in the vault, nothing kept unless a test says so. */
const backend = (over: Record<string, (args: any) => unknown> = {}) => ({
  finder_setup_state: () => ({ setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }),
  sync_status: () => ({ logged_in: true, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }),
  list_remote_tree: () => [],
  kept_unsynced_folder: () => null,
  kept_unsent_folder: () => null,
  ...over,
})

function open(name: string, b: Record<string, (args: any) => unknown>) {
  const m = mount('MacSettings.tsx', name, {
    backend: b as any,
    expand: true,
    props: { settings },
    bindings: baseBindings,
    hookModules: [useFinderSetupModule(), useKeptFoldersModule()],
  })
  mounted.push(m)
  return m
}

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
/** The hook and the rows read once their subscriptions land, so a render needs microtasks and a macrotask. */
const settle = async (m: Mounted) => { await m.flush(); await tick(); await m.flush(); await m.flush() }
const find = (m: Mounted, match: (el: TreeNode) => boolean) => m.elements().filter(match)
const buttons = (m: Mounted) => find(m, (el) => el.type === 'button').map((el) => textOf(el.props.children).trim()).filter(Boolean)
const press = async (m: Mounted, label: string) => {
  const hit = find(m, (el) => el.type === 'button' && textOf(el.props.children).trim() === label)
  if (hit.length !== 1) throw new Error(`expected one button "${label}", found ${hit.length}; buttons: ${buttons(m).join(' | ')}`)
  await hit[0].props.onClick()
  await settle(m)
}
/** Presses the first button with that label: ours, because ours renders first. */
const pressFirst = async (m: Mounted, label: string) => {
  const hit = find(m, (el) => el.type === 'button' && textOf(el.props.children).trim() === label)
  if (hit.length === 0) throw new Error(`no button "${label}"`)
  await hit[0].props.onClick()
  await settle(m)
}
function readable(node: any): string[] {
  if (Array.isArray(node)) return node.flatMap(readable)
  if (node == null || typeof node === 'boolean') return []
  if (typeof node !== 'object' || !node.props) return [String(node)]
  if (node.type === 'select' || node.type === 'svg') return []
  return readable(node.props.children)
}
const visibleText = (m: Mounted) => readable(expand(m.tree())).join(' | ')
/** The kept-folder rows: each status note that carries one of the two sentences, as a person reads it. */
const keptRows = (m: Mounted) =>
  find(m, (el) => el.props.role === 'status')
    .map((el) => readable(el).join(' | '))
    .filter((text) => text.includes(copy.K2_ROW) || text.includes(KEPT_FOLDER_ROW_SENTENCE))

const OURS = '/Users/sam/Library/Containers/io.beebeeb.app/Data/Documents/Changes not sent/2026-10-10 at 14.05.12'
const MACOS = '/Users/sam/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 14.05)'

describe('Settings › Sync: the kept folder of a sign-out (spec 2026-10-10 §9)', () => {
  /** F5. */
  test('both rows, ours first, each with its own Dismiss; ours has Show in Finder; neither says "your"', async () => {
    const m = open('SyncTab', backend({
      kept_unsent_folder: () => OURS,
      kept_unsynced_folder: () => MACOS,
      show_kept_unsent_folder: () => { throw new Error(copy.K3_TOAST) },
      dismiss_kept_unsent_folder: () => ({ cleared: true, current: null }),
    }))
    await settle(m)
    const text = visibleText(m)
    expect(text.indexOf(copy.K2_ROW)).toBeGreaterThanOrEqual(0)
    expect(text.indexOf(copy.K2_ROW)).toBeLessThan(text.indexOf(KEPT_FOLDER_ROW_SENTENCE))
    const rows = keptRows(m)
    expect(rows).toHaveLength(2)
    expect(rows[0]).toContain(OURS)
    expect(rows[1]).toContain(MACOS)
    for (const row of rows) expect(row).not.toMatch(/\byour\b/i)
    expect(buttons(m).filter((b) => b === copy.K2_DISMISS)).toHaveLength(2)
    expect(buttons(m)).toContain(copy.K2_SHOW)
    await press(m, copy.K2_SHOW)
    expect(m.calls.find((c) => c.name === 'show_kept_unsent_folder')?.args).toEqual({ path: OURS })
    expect(m.toasts.map((t) => t.message)).toEqual([copy.K3_TOAST])
    expect(visibleText(m)).toContain(copy.K2_ROW) // the row stays until Dismiss
    await pressFirst(m, copy.K2_DISMISS)
    expect(m.calls.find((c) => c.name === 'dismiss_kept_unsent_folder')?.args).toEqual({ path: OURS })
    expect(visibleText(m)).not.toContain(copy.K2_ROW)
    expect(visibleText(m)).toContain(KEPT_FOLDER_ROW_SENTENCE)
  })
})
