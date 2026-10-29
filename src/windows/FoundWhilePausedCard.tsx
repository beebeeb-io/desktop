/**
 * Settings › Sync: changes found while sync was paused (task 2267, Windows). Local items Beebeeb could not prove were
 * the account's own wait, held with the pause reason `found_while_paused`, until the person decides: "Upload to my
 * Beebeeb" (the primary action) or "Keep on this PC only", which asks inline first and then moves them out of the sync
 * folder. The words, the result lines and when the card shows are `foundWhilePaused.ts`; nothing here decides a
 * sentence. No command takes a path: the backend knows which items and which folder.
 *
 * Lead review round 3 of #176: the card reads what is held, by kind (`found_while_paused_review`), and both choices
 * wait for that read; Upload sends back exactly the ids it counted (P2-2, P3-1). Nothing takes the focus on its own.
 * A read is made when the held count needs one, at most once per summary the panel polls.
 *
 * Mounted always by the Sync panel and draws nothing while there is nothing to show, so a keep result survives the
 * poll that no longer reports the group (it stays until dismissed or the panel unmounts).
 */
import { useEffect, useRef, useState, type ReactNode } from 'react'
import {
  commandUnavailableLabel,
  foundWhilePausedKeep,
  foundWhilePausedReview,
  foundWhilePausedUpload,
  showKeptFolder,
} from '../desktopApi'
import {
  FOUND_WHILE_PAUSED_COPY,
  NOT_CONFIRMED_LINE,
  foundWhilePausedCardVisible,
  foundWhilePausedCount,
  heldKindsLine,
  keepConfirm,
  keepLabel,
  keepResultLines,
  waitingForDecisionLine,
  type FoundWhilePausedReview,
  type KeepResult,
} from './foundWhilePaused'
import type { SyncActivitySummary } from './syncActivityModel'
import { Card, NavIcon, T } from './ui'

const RED = 'var(--red)'
/** Text on a solid `--amber` fill (design.css): `T.ink` is near-white in the dark theme. */
const AMBER_INK = 'var(--amber-ink)'

export function FoundWhilePausedCard({
  summary,
  refresh,
}: {
  /** The Sync panel's last `sync_activity_summary` read. */
  summary: SyncActivitySummary | null | undefined
  /** The panel's own re-read of the summary (`useSyncActivitySummary().refresh`). */
  refresh: () => Promise<void>
}) {
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState<'upload' | 'keep' | 'show' | null>(null)
  const [kept, setKept] = useState<KeepResult | null>(null)
  // Inline by the lead's contract for task 2267, not a toast: the card is a decision that stays on screen until it is
  // made, and a failed Upload, Move or Show in Explorer is shown beside the buttons it concerns until the next press
  // (as the Updates card shows a failed downgrade). `at` puts it in the part of the card the button lives in.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [error, setError] = useState<{ text: string; at: 'decision' | 'result' } | null>(null)
  // What is held, by kind, and the ids Upload sends back; null until read (or after a decision changed what is held).
  const [review, setReview] = useState<FoundWhilePausedReview | null>(null)
  // A load failure (the read), shown inline: the choices wait for that read.
  const [reviewError, setReviewError] = useState<string | null>(null)
  // The summary the last read was made for: one read per poll at most, however the counts compare.
  const readFor = useRef<SyncActivitySummary | null | undefined>(undefined)

  const count = foundWhilePausedCount(summary)
  const visible = foundWhilePausedCardVisible(summary, { busy: busy !== null, kept })
  const needsReview = count > 0 && (review === null || review.op_ids.length !== count)

  // A card that went away (the changes were decided elsewhere) comes back at its first step, without an old error.
  useEffect(() => {
    if (visible) return
    setConfirming(false)
    setError(null)
    setReview(null)
    setReviewError(null)
    readFor.current = undefined
  }, [visible])

  useEffect(() => {
    if (!needsReview || busy !== null || readFor.current === summary) return
    readFor.current = summary
    let live = true
    void foundWhilePausedReview().then((result) => {
      if (!live) return
      if (result.ok) {
        setReview(result.value)
        setReviewError(null)
      } else {
        setReview(null)
        setReviewError(result.unsupported ? commandUnavailableLabel('found_while_paused_review') : result.reason)
      }
    })
    return () => { live = false }
  }, [needsReview, summary, busy])

  // After a decision what is held has changed: read it again.
  const readAgain = () => {
    setReview(null)
    readFor.current = undefined
  }

  const upload = async () => {
    if (busy || !review) return
    setBusy('upload')
    setError(null)
    const result = await foundWhilePausedUpload(review.op_ids)
    if (result.ok) readAgain()
    else setError({ text: result.unsupported ? commandUnavailableLabel('found_while_paused_upload') : result.reason, at: 'decision' })
    await refresh()
    setBusy(null)
  }

  const keep = async () => {
    if (busy) return
    setBusy('keep')
    setError(null)
    const result = await foundWhilePausedKeep()
    if (result.ok) setKept(result.value)
    else setError({ text: result.unsupported ? commandUnavailableLabel('found_while_paused_keep') : result.reason, at: 'decision' })
    setConfirming(false)
    readAgain()
    await refresh()
    setBusy(null)
  }

  const showFolder = async () => {
    if (busy) return
    setBusy('show')
    setError(null)
    const result = await showKeptFolder()
    if (!result.ok) setError({ text: result.unsupported ? commandUnavailableLabel('show_kept_folder') : result.reason, at: 'result' })
    setBusy(null)
  }

  const askToKeep = () => {
    setError(null)
    setConfirming(true)
  }

  const dismiss = () => {
    setKept(null)
    setError(null)
  }

  if (!visible) return null

  const anyBusy = busy !== null
  // Both choices wait for the read: Upload sends exactly what it counted, and the question names the service (if any)
  // that syncs the kept folder.
  const waiting = anyBusy || review === null
  const syncedBy = review?.synced_by ?? null
  const deciding = count > 0 || busy === 'upload' || busy === 'keep'
  const lines = kept ? keepResultLines(kept) : []
  const kinds = review ? heldKindsLine(review) : ''
  const alertStyle = { marginTop: 10, fontSize: 11.5, color: RED, lineHeight: 1.45, fontFamily: T.fontMono, wordBreak: 'break-word' as const }
  const errorLine = (at: 'decision' | 'result') =>
    error?.at === at ? (
      <div role="alert" style={alertStyle}>
        {error.text}
      </div>
    ) : null

  return (
    <Card style={{ padding: 0, marginBottom: 22, overflow: 'hidden' }}>
      {deciding && (
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 14, padding: '18px 20px' }}>
          <div style={{ width: 32, height: 32, borderRadius: 8, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="folder" size={14} color={T.ink2} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13.5, fontWeight: 600, color: T.ink, lineHeight: 1.4 }}>{FOUND_WHILE_PAUSED_COPY.title}</div>
            {count > 0 && (
              <div style={{ fontSize: 12, color: T.ink3, marginTop: 4, lineHeight: 1.45 }}>
                <div>{kinds || waitingForDecisionLine(count)}</div>
                <div style={{ marginTop: 2 }}>{NOT_CONFIRMED_LINE}</div>
              </div>
            )}
            {confirming ? (
              <div style={{ marginTop: 14, padding: '12px 14px', borderRadius: 8, background: T.paper2, border: `1px solid ${T.line}` }}>
                <div style={{ fontSize: 12, color: T.ink2, lineHeight: 1.5 }}>{keepConfirm(syncedBy)}</div>
                <div style={{ display: 'flex', flexWrap: 'wrap' as const, gap: 8, marginTop: 12 }}>
                  <CardButton primary busy={busy === 'keep'} disabled={anyBusy} onClick={keep}>{FOUND_WHILE_PAUSED_COPY.move}</CardButton>
                  <CardButton disabled={anyBusy} onClick={() => setConfirming(false)}>{FOUND_WHILE_PAUSED_COPY.cancel}</CardButton>
                </div>
              </div>
            ) : (
              <div style={{ display: 'flex', flexWrap: 'wrap' as const, gap: 8, marginTop: 14 }}>
                <CardButton primary busy={busy === 'upload'} disabled={waiting} onClick={upload}>{FOUND_WHILE_PAUSED_COPY.upload}</CardButton>
                <CardButton disabled={waiting} onClick={askToKeep}>{keepLabel(syncedBy)}</CardButton>
              </div>
            )}
            {count > 0 && reviewError !== null && !confirming && (
              <div role="alert" style={alertStyle}>
                {reviewError}
              </div>
            )}
            {errorLine('decision')}
          </div>
        </div>
      )}
      {lines.length > 0 && (
        <div style={{ padding: '14px 20px 16px', borderTop: deciding ? `1px solid ${T.line}` : 'none', background: T.paper2 }}>
          <div aria-live="polite">
            {lines.map((line, i) => (
              <div key={line.key} style={{ fontSize: 12, color: T.ink2, lineHeight: 1.5, marginTop: i === 0 ? 0 : 6 }}>
                {line.path ? (
                  <>
                    {line.path.before}
                    <span style={{ fontFamily: T.fontMono, fontSize: 11.5, color: T.ink, overflowWrap: 'anywhere' as const }}>{line.path.path}</span>
                    {line.path.after}
                  </>
                ) : line.text}
              </div>
            ))}
          </div>
          <div style={{ display: 'flex', flexWrap: 'wrap' as const, gap: 8, marginTop: 12 }}>
            {lines.some((line) => line.key === 'moved') && (
              <CardButton busy={busy === 'show'} disabled={anyBusy} onClick={showFolder}>{FOUND_WHILE_PAUSED_COPY.showInExplorer}</CardButton>
            )}
            <CardButton disabled={anyBusy} onClick={dismiss}>{FOUND_WHILE_PAUSED_COPY.dismiss}</CardButton>
          </div>
          {errorLine('result')}
        </div>
      )}
    </Card>
  )
}

/** The card's buttons: the step's primary action is amber (the one accent), every other one outlined. */
export function CardButton({
  children,
  onClick,
  primary = false,
  busy = false,
  disabled = false,
}: {
  children: ReactNode
  onClick: () => void
  primary?: boolean
  busy?: boolean
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-busy={busy || undefined}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        height: 32,
        padding: '0 14px',
        fontSize: 12.5,
        fontFamily: T.fontSans,
        fontWeight: 500,
        borderRadius: 6,
        border: `1px solid ${primary ? T.amberDeep : T.line2}`,
        background: primary ? T.amber : T.paper,
        // --amber does not change between themes; --amber-ink is the label colour that stays readable on it in both.
        color: primary ? AMBER_INK : T.ink,
        cursor: busy ? 'progress' : disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.6 : 1,
        whiteSpace: 'nowrap' as const,
        flexShrink: 0,
      }}
    >
      {children}
    </button>
  )
}
