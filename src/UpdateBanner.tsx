/**
 * UpdateBanner listens for the desktop updater event and surfaces it through
 * the shared toast system. The install command still gets a bounded timeout so
 * a stalled updater never leaves the action stuck in progress.
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command, openUrl } from './desktopApi'
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
  const [update, setUpdate] = useState<UpdatePayload | null>(null)
  const [installState, setInstallState] = useState<InstallState>('idle')
  // Paired with the installState machine (installState === 'error'); the banner persists until
  // dismissed or retried, so it is a durable state, not a transient notification.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [installError, setInstallError] = useState<string | null>(null)

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
  }, [])

  const releaseNotesUrl = useMemo(() => {
    if (!update) return null
    return update.release_notes_url ?? `https://github.com/beebeeb-io/desktop/releases/tag/desktop-v${encodeURIComponent(update.version)}`
  }, [update])

  useEffect(() => {
    if (!update || !releaseNotesUrl) return

    const installFailed = installState === 'error' && installError != null
    showToast({
      id: 'desktop-update',
      variant: installFailed ? 'error' : 'info',
      title: installFailed ? 'Update install failed' : `Version ${update.version} is available.`,
      message: (
        <>
          <span style={{ color: T.ink2 }}>Restart to apply the update.</span>
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
          {update.body && (
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
              {update.body}
            </span>
          )}
          {installFailed && (
            <span style={{ display: 'block', marginTop: 4, color: 'oklch(0.42 0.15 25)', lineHeight: 1.4 }}>
              Install failed: {installError}
            </span>
          )}
        </>
      ),
      action: {
        label: installState === 'installing' ? 'Installing...' : installFailed ? 'Try again' : 'Restart to update',
        onClick: handleInstall,
        disabled: installState === 'installing',
        ariaBusy: installState === 'installing',
      },
      durationMs: null,
    })
  }, [handleInstall, installError, installState, releaseNotesUrl, showToast, update])

  return null
}
