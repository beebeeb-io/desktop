// Batch B03's frames (design/desktop-v2/BATCHES.md). Owned by B03 only.
import type { ReactElement } from 'react'
import type { StorageSummary, SyncSummary } from '@beebeeb/shared/drive-ui'
import type { FrameRegistry } from './types'
import { Banner, bannerCopy, type BannerKind } from '../chrome/Banners'
import { CommandsDialog } from '../chrome/CommandsDialog'
import { EDITOR_COMMANDS, FILES_COMMANDS, NAVIGATION_COMMANDS } from '../chrome/commands'
import { EmptyFiles, GettingReadyStrip, NeedsConnection, PlainTile } from '../chrome/EmptyStates'
import { FrameWindow, ListBackdrop, STORAGE_240, SYNC_OK } from '../chrome/FrameWindow'
import { TaskbarStrip, TrayIcon, TrayMenu, TRAY_STATES } from '../chrome/TrayArt'
import { ToastView } from '../chrome/Toast'
import { ROOT_ITEMS } from '../source/fixtures'

const GB = 1024 ** 3
const TB = 1024 * GB
const noop = () => {}

function storageOf(usedGb: number): StorageSummary {
  return { usedBytes: usedGb * GB, quotaBytes: TB, filesBytes: usedGb * GB, photosBytes: 0, backupsBytes: 0 }
}

function bannerFrame(kind: BannerKind, o: { storage?: StorageSummary; sync?: SyncSummary } = {}): () => ReactElement {
  return () => {
    const dismissable = kind === 'sync-problem' || kind === 'backup-problem' || kind === 'storage-almost-full'
    return (
      <FrameWindow
        storage={o.storage}
        sync={o.sync}
        banner={<Banner kind={kind} copy={bannerCopy(kind, { storage: o.storage ?? null })} onAction={noop} onDismiss={dismissable ? noop : undefined} />}
      />
    )
  }
}

const OFFLINE: SyncSummary = { ...SYNC_OK, phase: 'offline' }

/** DW-0119: the grid with plain tiles where thumbnails have not arrived. */
function PlainGrid() {
  const files = ROOT_ITEMS.filter((i) => !i.isFolder)
  const size = (b: number) => (b >= 1024 ** 2 ? `${(b / 1024 ** 2).toFixed(1)} MB` : `${Math.round(b / 1024)} KB`)
  return (
    <div data-grid className="grid flex-1 grid-cols-4 content-start gap-4 overflow-hidden p-4">
      {files.map((f) => <PlainTile key={f.id} name={f.name} size={size(f.size ?? 0)} />)}
    </div>
  )
}

function Board({ children }: { children: React.ReactNode }) {
  return <div className="grid min-h-screen place-items-center bg-paper p-8 font-sans text-ink">{children}</div>
}

function TrayStates() {
  return (
    <Board>
      <div className="grid w-[960px] grid-cols-3 gap-6">
        {TRAY_STATES.map((s) => (
          <section key={s.state} data-tray-card={s.state} className="rounded-xl border border-line bg-paper-2 p-4">
            <h2 className="m-0 text-[13px] font-semibold text-ink">{s.label}</h2>
            <div className="mt-3 flex items-center gap-3">
              <TaskbarStrip state={s.state} mode="light" />
              <TaskbarStrip state={s.state} mode="dark" />
            </div>
            <div className="mt-3 flex items-end gap-4">
              <div className="rounded-md bg-[var(--tray-light-bg)] p-2"><TrayIcon state={s.state} mode="light" size={64} /></div>
              <div className="rounded-md bg-[var(--tray-dark-bg)] p-2"><TrayIcon state={s.state} mode="dark" size={64} /></div>
            </div>
            <p className="m-0 mt-3 font-mono text-[11px] leading-4 text-ink-3">{s.tooltip}</p>
            <p className="m-0 mt-1 text-[12px] leading-4 text-ink-3">{s.flyouts}</p>
          </section>
        ))}
      </div>
    </Board>
  )
}

function TrayMenuFrame({ paused }: { paused: boolean }) {
  return (
    <Board>
      <div className="flex flex-col items-start gap-1">
        <TrayMenu paused={paused} />
        <TaskbarStrip state={paused ? 'paused' : 'up-to-date'} mode="light" />
      </div>
    </Board>
  )
}

const commandsFrame = (lists: typeof FILES_COMMANDS, title = 'Beebeeb'): (() => ReactElement) => () => (
  <FrameWindow title={title} overlay={<CommandsDialog lists={lists} onClose={noop} />} />
)

export const FRAMES: FrameRegistry = {
  'DW-0101': bannerFrame('offline', { sync: OFFLINE }),
  'DW-0102': bannerFrame('sync-problem'),
  'DW-0103': bannerFrame('storage-almost-full', { storage: storageOf(950) }),
  'DW-0104': bannerFrame('storage-full', { storage: storageOf(1024) }),
  'DW-0105': bannerFrame('signed-out'),
  'DW-0106': bannerFrame('backup-problem'),
  'DW-0107': () => <FrameWindow toast={<ToastView message={{ kind: 'done', text: 'Link copied' }} />} />,
  'DW-0108': () => <FrameWindow toast={<ToastView message={{ kind: 'undo', text: 'Moved to Trash', onAction: noop }} />} />,
  'DW-0109': () => <FrameWindow toast={<ToastView message={{ kind: 'failed', text: "Couldn't rename. Check your connection.", onAction: noop }} />} />,
  'DW-0114': () => (
    <FrameWindow
      storage={storageOf(0)}
      sync={{ ...SYNC_OK, onDevice: 0, availableOffline: 0, onlineOnly: 0 }}
      counts={{ files: 0, shared: 0, photos: 0, trash: 0, 'sync-history': 0, 'available-offline': 0 }}
    >
      <EmptyFiles onAddFiles={noop} />
    </FrameWindow>
  ),
  'DW-0118': () => (
    <FrameWindow strip={<GettingReadyStrip loading={{ done: 18240, total: 40112, minutesLeft: 1, thumbnails: 'waiting' }} />}>
      <ListBackdrop />
    </FrameWindow>
  ),
  'DW-0119': () => <FrameWindow><PlainGrid /></FrameWindow>,
  'DW-0121': () => (
    <FrameWindow
      sync={OFFLINE}
      storage={STORAGE_240}
      banner={<Banner kind="offline" copy={bannerCopy('offline')} />}
      aside={<NeedsConnection fileName="Contract draft.pdf" onTryAgain={noop} onClose={noop} />}
    />
  ),
  'DW-1230': () => <TrayStates />,
  'DW-1231': () => <TrayMenuFrame paused={false} />,
  'DW-1231b': () => <TrayMenuFrame paused />,
  'DW-1232': commandsFrame(FILES_COMMANDS),
  'DW-1233': commandsFrame(NAVIGATION_COMMANDS),
  'DW-1234': commandsFrame(EDITOR_COMMANDS, 'Meeting notes.md - Beebeeb'),
}
