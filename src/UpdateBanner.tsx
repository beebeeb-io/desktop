import ManualUpdateFeedback from './ManualUpdateFeedback'
import { useCapabilities, canInstallUpdate } from './capabilities'
/**
 * UpdateBanner listens for the desktop updater event and surfaces it through
 * the shared toast system. The install command still gets a bounded timeout so
 * a stalled updater never leaves the action stuck in progress.
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command, openUrl } from './desktopApi'
import { updateAvailableLine } from './macUpdateCopy'
import { releasePageUrl, updateOffer } from './macUpdateOffer'
import { T, useToast } from './windows/ui'

interface UpdatePayload {
  version: string
  body: string
  channel?: 'stable' | 'beta' | 'alpha'
  release_notes_url?: string
}

type InstallState = 'idle' | 'installing' | 'error'

export default function UpdateBanner() {
  const { showToast } = useToast()
  const caps = useCapabilities()
  const installSupported = canInstallUpdate(caps)
  // Task 2176: a Mac downloads its update; the App Sandbox denies the in-place install.
  const downloadOnly = caps?.host_os === 'macos'
  const [update, setUpdate] = useState<UpdatePayload | null>(null)
  const [installState, setInstallState] = useState<InstallState>('idle')
  const [installError, setInstallError] = useState<string | null>(null)
  // What a Mac is offered for it: the newest stable image for a stable update, that release's page for another channel.
  // The toast's effect depends on the offer's three strings and not on the object: a new object every render would show
  // the toast again on every render, and `App.tsx`'s 5 s status poll renders this banner (review P2-A).
  const offer = update && downloadOnly ? updateOffer(update.channel, update.version) : null
  const offerLabel = offer?.label ?? null
  const offerUrl = offer?.url ?? null
  const offerHint = offer?.hint ?? null

  useEffect(() => {
    let cancelled = false

    const unlisten = listen<UpdatePayload>('update-available', (event) => {
      if (cancelled) return
      setUpdate(event.payload)
      setInstallState('idle')
      setInstallError(null)
    })

    return () => {
      cancelled = true
      void unlisten.then((fn) => fn())
    }
  }, [])

  const handleInstall = useCallback(async () => {
    if (!installSupported) return
    setInstallState('installing')
    setInstallError(null)

    let timeoutId: ReturnType<typeof setTimeout> | undefined
    const timeoutPromise = new Promise<'timeout'>((resolve) => {
      timeoutId = setTimeout(() => resolve('timeout'), 30_000)
    })

    try {
      const raceResult = await Promise.race([
        command<void>('install_update'),
        timeoutPromise,
      ])

      if (raceResult === 'timeout') {
        setInstallState('error')
        setInstallError('Install timed out; try again.')
        return
      }

      clearTimeout(timeoutId)
      if (raceResult.ok) return
      setInstallState('error')
      setInstallError(raceResult.reason)
    } finally {
      clearTimeout(timeoutId)
    }
  }, [installSupported])

  // The same goes for the update itself: the updater announces it again with every 4-hourly check, each time as a new
  // payload object, and a person who dismissed the toast for this version has answered it (review P3-1). So the toast's
  // effect lists the update's strings, never the object.
  const version = update?.version ?? null
  const body = update?.body ?? ''
  const releaseNotesUrl = useMemo(() => {
    if (version === null) return null
    return update?.release_notes_url ?? releasePageUrl(version)
  }, [update?.release_notes_url, version])

  useEffect(() => {
    if (version === null || !releaseNotesUrl) return

    const installFailed = installState === 'error' && installError != null
    showToast({
      id: 'desktop-update',
      variant: installFailed ? 'error' : 'info',
      title: installFailed ? 'Update install failed' : updateAvailableLine(version),
      message: (
        <>
          <span style={{ color: T.ink2 }}>{offerHint !== null ? offerHint : installSupported ? 'Restart to apply the update.' : caps?.update_format === 'package_manager' ? 'Update with your package manager.' : 'Download an installer to update this installation.'}</span>
          {' '}
          <button
            type="button"
            onClick={() => void openUrl(releaseNotesUrl)}
            style={{
              border: 'none',
              background: 'transparent',
              padding: 0,
              color: T.ink,
              font: 'inherit',
              fontWeight: 700,
              cursor: 'pointer',
              textDecoration: 'underline',
              textUnderlineOffset: 2,
            }}
          >
            Release notes
          </button>
          {body && (
            <span
              style={{
                display: 'block',
                marginTop: 4,
                fontSize: 11.5,
                color: T.ink3,
                fontFamily: T.fontMono,
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
              }}
            >
              {body}
            </span>
          )}
          {installFailed && (
            <span style={{ display: 'block', marginTop: 4, color: 'oklch(0.42 0.15 25)', lineHeight: 1.4 }}>
              Install failed: {installError}
            </span>
          )}
        </>
      ),
      action: offerLabel !== null && offerUrl !== null ? { label: offerLabel, onClick: () => { void openUrl(offerUrl) } } : installSupported ? {
        label: installState === 'installing' ? 'Installing...' : installFailed ? 'Try again' : 'Restart to update',
        onClick: handleInstall,
        disabled: installState === 'installing',
        ariaBusy: installState === 'installing',
      } : { label: 'Downloads', onClick: () => { void openUrl('https://beebeeb.io/download') } },
      durationMs: null,
    })
  }, [body, caps?.update_format, offerHint, offerLabel, offerUrl, installSupported, handleInstall, installError, installState, releaseNotesUrl, showToast, version])

  return <ManualUpdateFeedback />
}
