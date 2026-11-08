/**
 * Activity › "Not sent" (task 2409; Guus: "Put it in Activity"): every change on this PC that has not reached Beebeeb,
 * one row each, with why it is not sent and the one action there is for it. "Not sent N" on Home and the tray's
 * waiting, held and given-up lines open this section.
 *
 * Rows follow the Activity "In progress" card (design/ld0105-needs-attention.html): the file-type icon, the name in
 * mono with its folder under it, the reason line (warn text when it needs the person), and one neutral action button.
 * Amber appears nowhere: this is the queue's state, not encryption's.
 *
 * The list carries names, so it is read only while this section is mounted and its window on screen (task 2243), and
 * only while the vault is unlocked; it is paged by {@link UNSENT_PAGE_SIZE} with "Show more".
 *
 * Task 2414: the same section, rows and layout serve Activity › "Not downloaded" (`list="not-downloaded"`): a pin's
 * downloads that are paused or out of attempts, which "Not downloaded N" on Home and the tray's line open.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { command, commandUnavailableLabel, forceReauth, openUrl } from '../../desktopApi'
import { T, Card, NavIcon, useToast } from '../ui'
import { FileGlyph, getFileType, FILE_COLOR } from '../fileGlyph'
import { pollWhileVisible } from '../visiblePoll'
import {
  UNSENT_PAGE_SIZE,
  baseName,
  folderOf,
  isLockedRefusal,
  keepRowStates,
  kindWord,
  loadUnsentChanges,
  notDownloadedAction,
  notDownloadedReason,
  queuedAgo,
  retryRefusal,
  retryUnsentChange,
  unsentAction,
  unsentReason,
  type RowState,
  type UnsentAction,
  type UnsentChange,
  type UnsentChangesPage,
} from '../unsentChanges'

/** How often the open section re-reads the list. */
export const NOT_SENT_POLL_MS = 5000

/** The two lists, each its anchor (the one Home's figure and the tray's lines open), its read and its words. */
export const QUEUE_LISTS = {
  'not-sent': {
    anchor: 'not-sent',
    command: 'unsent_changes',
    icon: 'cloud',
    heading: 'Not sent',
    subtitle: 'Changes on this PC that haven’t reached Beebeeb yet.',
    reading: 'Reading the changes not sent…',
    failed: 'Couldn’t read the changes not sent',
    locked: 'Beebeeb is locked. Unlock it to see which changes these are.',
    empty: 'Every change on this PC has been sent.',
    counted: 'not sent',
  },
  'not-downloaded': {
    anchor: 'not-downloaded',
    command: 'not_downloaded',
    icon: 'download',
    heading: 'Not downloaded',
    subtitle: 'Files and folders you chose to keep on this PC that didn’t finish downloading.',
    reading: 'Reading the files not downloaded…',
    failed: 'Couldn’t read the files not downloaded',
    locked: 'Beebeeb is locked. Unlock it to see which items these are.',
    empty: 'Everything you chose to keep on this PC is downloaded.',
    counted: 'not downloaded',
  },
} as const
export type QueueList = keyof typeof QUEUE_LISTS

const WARN_COLOR = 'var(--warn-text)'

type Read =
  | { state: 'reading' }
  | { state: 'locked' }
  | { state: 'failed'; reason: string }
  | { state: 'loaded'; page: UnsentChangesPage }

const actionButton: React.CSSProperties = {
  padding: '5px 12px',
  fontSize: 11.5,
  fontFamily: 'var(--font-sans)',
  fontWeight: 500,
  color: 'var(--ink)',
  background: 'var(--paper-2)',
  border: '1px solid var(--line)',
  borderRadius: 7,
  cursor: 'pointer',
  textDecoration: 'none',
  display: 'inline-block',
}

export function NotSentSection({
  list = 'not-sent',
  focus = 0,
  onOpenSyncSettings,
}: {
  /** Which list: "Not sent" (default) or "Not downloaded" (task 2414). */
  list?: QueueList
  /** Bumped each time a link opens this section: it scrolls into view and takes the focus. */
  focus?: number
  onOpenSyncSettings?: () => void
}) {
  const words = QUEUE_LISTS[list]
  const [read, setRead] = useState<Read>({ state: 'reading' })
  const [shown, setShown] = useState(UNSENT_PAGE_SIZE)
  const [rows, setRows] = useState<Record<string, RowState>>({})
  const { showToast } = useToast()
  const heading = useRef<HTMLHeadingElement>(null)
  const latest = useRef(0)

  const refresh = useCallback(async () => {
    const ticket = ++latest.current
    const result = await loadUnsentChanges(shown, command, words.command)
    if (ticket !== latest.current) return
    if (result.ok) {
      setRead({ state: 'loaded', page: result.value })
      // An action in flight keeps its row state, and a Try again's "Queued" until the read shows the change moving.
      const items = result.value.items
      setRows((all) => keepRowStates(all, items))
    } else if (isLockedRefusal(result.reason)) setRead({ state: 'locked' })
    else setRead({ state: 'failed', reason: result.unsupported ? commandUnavailableLabel(words.command) : result.reason })
  }, [shown, words.command])

  useEffect(() => pollWhileVisible(refresh, NOT_SENT_POLL_MS), [refresh])

  useEffect(() => {
    if (focus <= 0) return
    heading.current?.scrollIntoView?.({ block: 'start' })
    heading.current?.focus({ preventScroll: true })
  }, [focus])

  const setRow = (id: string, state: RowState | null) =>
    setRows((all) => {
      const next = { ...all }
      if (state) next[id] = state
      else delete next[id]
      return next
    })

  const act = async (item: UnsentChange, action: UnsentAction) => {
    if (action.kind === 'add_storage') {
      await openUrl(action.href)
      return
    }
    if (action.kind === 'open_sync_settings') {
      onOpenSyncSettings?.()
      return
    }
    setRow(item.id, 'busy')
    if (action.kind === 'sign_in_again') {
      const result = await forceReauth()
      setRow(item.id, null)
      if (!result.ok) showToast({ variant: 'error', title: 'Couldn’t open sign-in', message: result.reason })
      return
    }
    const result = await retryUnsentChange(item.id)
    setRow(item.id, result.ok ? 'queued' : null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t try it again',
        message: result.unsupported ? commandUnavailableLabel('retry_unsent_change') : retryRefusal(result.reason),
      })
    }
    await refresh()
  }

  const unlock = async () => {
    const result = await command<void>('unlock_vault')
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t unlock Beebeeb',
        message: result.unsupported ? commandUnavailableLabel('unlock_vault') : result.reason,
      })
    }
    await refresh()
  }

  const page = read.state === 'loaded' ? read.page : null
  const total = page?.total ?? null

  return (
    <section id={words.anchor} aria-labelledby={`${words.anchor}-heading`} style={{ marginBottom: 16, scrollMarginTop: 16 }}>
      <Card style={{ padding: 0, overflow: 'hidden' }}>
        <div style={{ padding: '12px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
          <NavIcon name={words.icon} size={14} color={T.ink2} />
          <div style={{ minWidth: 0, flex: 1 }}>
            <h2 id={`${words.anchor}-heading`} ref={heading} tabIndex={-1} style={{ margin: 0, fontSize: 12.5, fontWeight: 600, color: T.ink, outline: 'none' }}>
              {words.heading}
            </h2>
            <div style={{ fontSize: 11, color: T.ink3 }}>{words.subtitle}</div>
          </div>
          {total !== null && (
            <span aria-label={`${total} ${words.counted}`} style={{ fontSize: 12, fontFamily: T.fontMono, color: T.ink2 }}>
              {total.toLocaleString('en-US')}
            </span>
          )}
        </div>

        {read.state === 'reading' && <Line>{words.reading}</Line>}
        {read.state === 'failed' && <Line warn>{words.failed}: {read.reason}</Line>}
        {read.state === 'locked' && (
          <div style={{ padding: '14px 20px', display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' as const }}>
            <span style={{ fontSize: 12, color: T.ink2, flex: 1, minWidth: 200 }}>{words.locked}</span>
            <button type="button" style={actionButton} onClick={() => void unlock()}>Unlock</button>
          </div>
        )}
        {page && page.items.length === 0 && <Line>{words.empty}</Line>}

        {page?.items.map((item, i) => (
          <NotSentRow
            key={item.id}
            item={item}
            page={page}
            last={i === page.items.length - 1 && page.items.length >= page.total}
            row={rows[item.id] ?? null}
            downloads={list === 'not-downloaded'}
            onAct={(action) => void act(item, action)}
          />
        ))}

        {page && page.items.length < page.total && (
          <div style={{ padding: '10px 20px', borderTop: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 12 }}>
            <button type="button" style={actionButton} onClick={() => setShown((n) => n + UNSENT_PAGE_SIZE)}>
              Show more
            </button>
            <span style={{ fontSize: 11, color: T.ink3, fontFamily: T.fontMono }}>
              {page.items.length.toLocaleString('en-US')} of {page.total.toLocaleString('en-US')}
            </span>
          </div>
        )}
      </Card>
    </section>
  )
}

function Line({ children, warn = false }: { children: React.ReactNode; warn?: boolean }) {
  return <div style={{ padding: '14px 20px', fontSize: 12, color: warn ? WARN_COLOR : T.ink3 }}>{children}</div>
}

function NotSentRow({
  item,
  page,
  last,
  row,
  downloads = false,
  onAct,
}: {
  item: UnsentChange
  page: UnsentChangesPage
  last: boolean
  row: RowState | null
  /** A "Not downloaded" row (task 2414): its own words and action. */
  downloads?: boolean
  onAct: (action: UnsentAction) => void
}) {
  const name = baseName(item.path) || 'An item'
  const folder = folderOf(item.path)
  const type = getFileType(name)
  const reason = downloads ? notDownloadedReason(item) : unsentReason(item, { engineState: page.engine_state, paused: page.paused })
  // A Try again that just worked reads "Queued" until a read shows it moving.
  const done = row === 'queued'
  const action = done ? null : downloads ? notDownloadedAction(item) : unsentAction(item)
  const fraction = item.sending && item.sending.total > 0 ? Math.min(1, item.sending.done / item.sending.total) : null
  return (
    <div
      data-unsent-id={item.id}
      style={{ padding: '11px 20px', display: 'flex', alignItems: 'flex-start', gap: 14, borderBottom: last ? 'none' : `1px solid ${T.line}` }}
    >
      <div style={{ width: 28, height: 28, borderRadius: 6, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
        {item.kind === 'folder' ? <NavIcon name="folder" size={13} color={T.ink3} /> : <FileGlyph type={type} size={13} color={FILE_COLOR[type]} />}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, minWidth: 0 }}>
          <span title={item.path} style={{ fontSize: 13, fontWeight: 500, color: T.ink, whiteSpace: 'nowrap' as const, overflow: 'hidden', textOverflow: 'ellipsis', fontFamily: T.fontMono, minWidth: 0 }}>
            {name}
          </span>
          <span style={{ fontSize: 9.5, fontFamily: T.fontMono, color: T.ink3, border: `1px solid ${T.line2}`, borderRadius: 4, padding: '0 4px', flexShrink: 0 }}>
            {item.backup ? `Backup · ${kindWord(item.kind)}` : kindWord(item.kind)}
          </span>
        </div>
        {folder && (
          <div title={folder} style={{ fontSize: 11, color: T.ink4, fontFamily: T.fontMono, marginTop: 1, whiteSpace: 'nowrap' as const, overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {folder}
          </div>
        )}
        <div style={{ fontSize: 11, color: !done && reason.attention ? WARN_COLOR : T.ink3, marginTop: 3, lineHeight: 1.5 }}>
          {done ? 'Queued' : reason.text}
        </div>
        {fraction !== null && (
          <div style={{ marginTop: 7, height: 3, borderRadius: 999, background: T.paper3, overflow: 'hidden' }}>
            <div style={{ width: `${Math.round(fraction * 100)}%`, height: '100%', background: T.ink3, borderRadius: 999 }} />
          </div>
        )}
        {action && (
          <div style={{ marginTop: 8 }}>
            {action.kind === 'add_storage' || action.kind === 'open_sync_settings' ? (
              // Places, not actions: links (brand rule). Add storage opens the billing page in the browser; the other
              // opens Settings › Sync in this window, where held changes are decided.
              <a
                href={action.kind === 'add_storage' ? action.href : '#settings-sync'}
                style={actionButton}
                onClick={(event) => {
                  event.preventDefault()
                  onAct(action)
                }}
              >
                {action.label}
              </a>
            ) : (
              <button type="button" style={{ ...actionButton, opacity: row === 'busy' ? 0.6 : 1 }} disabled={row === 'busy'} onClick={() => onAct(action)}>
                {row === 'busy' ? (action.kind === 'retry' ? 'Trying again…' : `${action.label}…`) : action.label}
              </button>
            )}
          </div>
        )}
      </div>
      <span style={{ fontSize: 11, color: T.ink3, fontFamily: T.fontMono, whiteSpace: 'nowrap' as const, flexShrink: 0 }}>
        {queuedAgo(item.queued_at)}
      </span>
    </div>
  )
}
