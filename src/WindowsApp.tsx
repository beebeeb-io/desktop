// The Windows main app window: its own sidebar and views, sharing the banners, search and version
// overlays with the compact shell. Capability gating stays in `capabilities.tsx`.

import { useRegionLabel } from './windows/useRegion'
import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command, loadSyncStatus, openUrl, accountUsage, type BillingUsage, type StorageSummary, type SyncStatus } from './desktopApi'
import { useCapabilities, supportsRoute, CapabilityAlternative } from './capabilities'
import UpdateBanner from './UpdateBanner'
import AuthExpiredBanner from './AuthExpiredBanner'
import DesktopQuickSearch, { DesktopQuickSearchTrigger } from './DesktopQuickSearch'
import DesktopVersionHistory, { DesktopVersionHistoryTrigger } from './DesktopVersionHistory'
import { T, NavIcon } from './windows/ui'
import KnownFolderOnboarding from './windows/KnownFolderOnboarding'
import AccountView from './windows/views/AccountView'
import InsightsView from './windows/views/InsightsView'
import BandwidthView from './windows/views/BandwidthView'
import SelectiveSyncView from './windows/views/SelectiveSyncView'
import DevicesView from './windows/views/DevicesView'
import SecurityView from './windows/views/SecurityView'
import ActivityView from './windows/views/ActivityView'
import SettingsView from './windows/views/SettingsView'
import { pollWhileVisible } from './windows/visiblePoll'
import { NavId, WINDOWS_APP_NAV_EVENT, WINDOWS_APP_NAV_STORAGE_KEY, navTargetFromString, initialNavTarget, ALWAYS_ACCESSIBLE, NAV_SECTIONS, type NavTarget } from './WindowsApp/nav'
import { SignedOutGate, PlaceholderView } from './WindowsApp/shell'
import { HomeView } from './WindowsApp/homeView'
import { KF_ONBOARDING_REOPEN_EVENT, KF_BACKUP_CHANGED_EVENT, FilesView } from './WindowsApp/filesView'
import { TrashView } from './WindowsApp/trashView'
import { StorageWidget } from './WindowsApp/storageWidget'

// ── Root component ──────────────────────────────────────────────────────────

export default function WindowsApp() {
  const regionLabel = useRegionLabel()
  const caps = useCapabilities()
  // Task 2409: a link to Activity › Not sent opens the route and bumps this, which scrolls the section into view.
  const [initialTarget] = useState<NavTarget>(() => initialNavTarget())
  const [activeNav, setActiveNav] = useState<NavId>(initialTarget.nav)
  const [notSentFocus, setNotSentFocus] = useState(initialTarget.section === 'not-sent' ? 1 : 0)
  // Task 2414: the same for Activity › Not downloaded.
  const [notDownloadedFocus, setNotDownloadedFocus] = useState(initialTarget.section === 'not-downloaded' ? 1 : 0)
  const openTarget = (target: NavTarget) => {
    setActiveNav(target.nav)
    if (target.section === 'not-sent') setNotSentFocus((n) => n + 1)
    if (target.section === 'not-downloaded') setNotDownloadedFocus((n) => n + 1)
  }
  // A link's focus lasts while Activity stays open: a later visit from the sidebar is Activity as before (the section
  // then shows only while something is not sent, and is not scrolled to).
  useEffect(() => {
    if (activeNav !== 'activity') {
      setNotSentFocus(0)
      setNotDownloadedFocus(0)
    }
  }, [activeNav])
  const [searchOpen, setSearchOpen] = useState(false)
  const [versionHistoryOpen, setVersionHistoryOpen] = useState(false)
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [usage, setUsage] = useState<BillingUsage | null>(null)
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  // Avoid refetching billing usage every poll once we have it.
  const usageLoaded = useRef(false)

  // Known-folder backup onboarding (task 0804). `mode` drives the one mounted
  // instance: 'auto' = self-gated first-run (shows once when never seen + no
  // folders backed up), 'forced' = a manual "Set up backup" re-open, 'closed' =
  // not mounted. We start in 'auto'; the component self-closes (via onClose)
  // when it's not a genuine first-run, so the overlay never flashes off-Windows
  // or for returning users. A bumping `key` remounts it on a forced re-open so
  // its internal load effect re-runs.
  const [kfOnboarding, setKfOnboarding] = useState<'auto' | 'forced' | 'closed'>('auto')
  const kfOnboardingKey = useRef(0)

  useEffect(() => {
    let cancelled = false
    const unlisten = listen<string>(WINDOWS_APP_NAV_EVENT, (event) => {
      if (cancelled) return
      const target = navTargetFromString(event.payload)
      if (target != null) openTarget(target)
    })

    const onStorage = (event: StorageEvent) => {
      if (event.key !== WINDOWS_APP_NAV_STORAGE_KEY) return
      const target = navTargetFromString(event.newValue)
      if (target != null) openTarget(target)
      try {
        window.localStorage.removeItem(WINDOWS_APP_NAV_STORAGE_KEY)
      } catch {
        // Best effort only; the Tauri event path above is the primary route
        // for already-open main-app windows.
      }
    }
    window.addEventListener('storage', onStorage)

    return () => {
      cancelled = true
      void unlisten.then((fn) => fn())
      window.removeEventListener('storage', onStorage)
    }
  }, [])

  // Manual re-open from the Manage-backup panel ("Set up backup").
  useEffect(() => {
    const onReopen = () => {
      kfOnboardingKey.current += 1
      setKfOnboarding('forced')
    }
    window.addEventListener(KF_ONBOARDING_REOPEN_EVENT, onReopen)
    return () => window.removeEventListener(KF_ONBOARDING_REOPEN_EVENT, onReopen)
  }, [])

  // Poll sync status (drives the auth gate + Home metrics), while the window is on screen (task 2243: closing the
  // window hides it, and each read opens the state DB).
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const next = await loadSyncStatus()
      if (!cancelled) setStatus(next)
    }
    const stop = pollWhileVisible(refresh, 5000)
    return () => { cancelled = true; stop() }
  }, [])

  const loggedIn = status?.logged_in ?? false

  // Load billing usage once signed in (storage widget + Home). On transient
  // failure we retry with EXPONENTIAL BACKOFF rather than a tight 4s loop — the
  // engine may not have a valid session ready on the very first
  // `logged_in: true` tick, so a single fire-and-forget call can strand the
  // widget at the skeleton state, but a fixed 4s loop hammers the shared billing
  // endpoint (which 429s under load) for as long as the session stays unready.
  //
  // Backoff: 4s → 8s → 16s → … capped at 60s. We STOP entirely once usage loads
  // (usageLoaded short-circuits the effect) and after a bounded number of
  // attempts, so a persistently-failing endpoint isn't polled forever. The
  // local-mirror fallback (desktop_storage_summary) still fills in a partial
  // signal on the way.
  useEffect(() => {
    if (!loggedIn) return
    // If we already have data, don't re-fetch on every status poll re-render.
    if (usageLoaded.current) return

    let cancelled = false
    let retryTimer: ReturnType<typeof window.setTimeout> | undefined
    let tries = 0

    const BASE_DELAY_MS = 4000
    const MAX_DELAY_MS = 60_000
    const MAX_TRIES = 8 // ~4+8+16+32+60+60+60s of backoff, then give up quietly

    const attempt = async () => {
      const r = await accountUsage()
      if (cancelled) return
      if (r.ok) {
        setUsage(r.value)
        usageLoaded.current = true
        return // loaded — stop retrying
      }
      // accountUsage() failed — try the local mirror for a partial signal.
      const s = await command<StorageSummary>('desktop_storage_summary')
      if (cancelled) return
      if (s.ok) {
        setStorage(s.value)
        // storage is a partial fallback; keep retrying accountUsage() (with
        // backoff) so the progress bar eventually shows accurate billing quota.
      }
      // Both failed (engine session not ready / endpoint rate-limited) —
      // schedule a backed-off retry so the widget recovers without hammering.
      tries += 1
      if (tries >= MAX_TRIES) return // give up quietly rather than loop forever
      const delay = Math.min(BASE_DELAY_MS * 2 ** (tries - 1), MAX_DELAY_MS)
      retryTimer = window.setTimeout(attempt, delay)
    }

    void attempt()

    return () => {
      cancelled = true
      if (retryTimer !== undefined) window.clearTimeout(retryTimer)
    }
  }, [loggedIn])

  // When signed out, snap back to an always-accessible view. Only once the first status has answered (task 2409): before
  // it, `loggedIn` is false for everyone, and a window opened on a route (the tray's "Not sent" lines open
  // Activity › Not sent) was sent Home before it knew. Until then the gate below shows instead of the route.
  useEffect(() => {
    if (status !== null && !loggedIn && !ALWAYS_ACCESSIBLE.has(activeNav)) setActiveNav('home')
  }, [status, loggedIn, activeNav])

  const openSignIn = () => { void command<void>('open_onboarding_window') }
  const openUpgrade = () => { void openUrl('https://app.beebeeb.io/billing') }

  const filteredSections = NAV_SECTIONS.map((section) => ({
    ...section,
    items: section.items.filter((item) => supportsRoute(caps, item.id) && (loggedIn || ALWAYS_ACCESSIBLE.has(item.id))),
  })).filter((section) => section.items.length > 0)

  const renderContent = () => {
    if (!supportsRoute(caps, activeNav)) return <CapabilityAlternative />
    if (!loggedIn && !ALWAYS_ACCESSIBLE.has(activeNav)) {
      return <SignedOutGate onOpenSignIn={openSignIn} />
    }
    switch (activeNav) {
      case 'home':
        return loggedIn ? (
          <HomeView status={status} usage={usage} storage={storage} onOpenList={(section) => openTarget({ nav: 'activity', section })} />
        ) : (
          <SignedOutGate onOpenSignIn={openSignIn} />
        )
      case 'files':
        return <FilesView status={status} />
      case 'trash':
        return <TrashView />
      case 'settings':
        return <SettingsView status={status} onOpenSignIn={openSignIn} />
      case 'account':
        return <AccountView />
      case 'insights':
        return <InsightsView />
      case 'devices':
        return <DevicesView />
      case 'security':
        return <SecurityView />
      case 'activity':
        return <ActivityView notSentFocus={notSentFocus} notDownloadedFocus={notDownloadedFocus} onOpenSettings={() => setActiveNav('settings')} />
      case 'bandwidth':
        return <BandwidthView />
      case 'selective-sync':
        return <SelectiveSyncView />
      default:
        return <PlaceholderView navId={activeNav} />
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100vh', fontFamily: T.fontSans, color: T.ink, background: T.paper, overflow: 'hidden' }}>
      {/* Keyframes for the skeleton shimmer + the refresh-icon spin (global,
          inline — so any view can use them without mounting UpdateBanner). */}
      <style>{'@keyframes bb-shimmer { 0% { background-position: 200% 0; } 100% { background-position: -200% 0; } } @keyframes bb-spin { to { transform: rotate(360deg); } }'}</style>
      <UpdateBanner />
      <AuthExpiredBanner authExpired={status?.auth_expired ?? false} />
      <DesktopQuickSearch
        open={searchOpen}
        onOpen={() => setSearchOpen(true)}
        onClose={() => setSearchOpen(false)}
      />
      <DesktopVersionHistory
        open={versionHistoryOpen}
        onOpen={() => setVersionHistoryOpen(true)}
        onClose={() => setVersionHistoryOpen(false)}
      />

      {/* Known-folder backup onboarding (task 0804). Only meaningful once signed
          in (it enables backup into the signed-in vault). 'auto' self-gates on
          the seen flag + empty backup set inside the component; 'forced' is an
          explicit re-open from the Manage-backup panel. */}
      {loggedIn && kfOnboarding !== 'closed' && (
        <KnownFolderOnboarding
          key={kfOnboardingKey.current}
          forced={kfOnboarding === 'forced'}
          onClose={(enabledAny) => {
            setKfOnboarding('closed')
            if (enabledAny) window.dispatchEvent(new CustomEvent(KF_BACKUP_CHANGED_EVENT))
          }}
        />
      )}
      <div style={{ display: 'grid', gridTemplateColumns: '232px 1fr', flex: 1, overflow: 'hidden' }}>
        {/* Sidebar */}
        <div style={{ background: T.paper2, borderRight: `1px solid ${T.line}`, padding: '16px 10px 0', overflow: 'auto', display: 'flex', flexDirection: 'column' }}>
          {/* Brand */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px 14px' }}>
            <div style={{ width: 24, height: 24, borderRadius: 6, background: T.amber, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <span style={{ fontWeight: 800, fontSize: 12, color: T.ink, lineHeight: 1 }}>b</span>
            </div>
            <span style={{ fontSize: 13, fontWeight: 600, letterSpacing: '-0.01em', color: T.ink }}>beebeeb.io</span>
          </div>

          <div style={{ padding: '0 2px 10px' }}>
            <DesktopQuickSearchTrigger onOpen={() => setSearchOpen(true)} />
            <div style={{ height: 7 }} />
            <DesktopVersionHistoryTrigger onOpen={() => setVersionHistoryOpen(true)} />
          </div>

          {/* Nav sections */}
          {filteredSections.map((section) => (
            <div key={section.heading} style={{ marginBottom: 10 }}>
              <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, padding: '4px 10px', marginBottom: 2 }}>
                {section.heading}
              </div>
              {section.items.map((item) => {
                const active = item.id === activeNav
                return (
                  <button
                    key={item.id}
                    onClick={() => setActiveNav(item.id)}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                      width: '100%',
                      padding: '7px 10px',
                      borderRadius: 6,
                      fontSize: 12.5,
                      fontWeight: active ? 600 : 400,
                      color: active ? T.ink : T.ink2,
                      background: active ? T.paper : 'transparent',
                      border: active ? `1px solid ${T.line}` : '1px solid transparent',
                      borderLeft: active ? `3px solid ${T.amberDeep}` : '3px solid transparent',
                      cursor: 'pointer',
                      textAlign: 'left' as const,
                      fontFamily: T.fontSans,
                      marginBottom: 2,
                      transition: 'background 100ms',
                    }}
                  >
                    <NavIcon name={item.icon} size={12} color={active ? T.amberDeep : T.ink3} />
                    {item.label}
                  </button>
                )
              })}
            </div>
          ))}

          {/* Storage widget (auth-gated content; shown only when signed in) */}
          {loggedIn ? (
            <StorageWidget usage={usage} storage={storage} onUpgrade={openUpgrade} />
          ) : (
            <div style={{ marginTop: 'auto', padding: '14px 12px 10px', borderTop: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 6 }}>
              <NavIcon name="shield" size={11} color={T.amberDeep} />
              <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, textTransform: 'uppercase' as const, letterSpacing: '0.04em' }}>
                {regionLabel}
              </span>
            </div>
          )}
        </div>

        {/* Content */}
        {renderContent()}
      </div>
    </div>
  )
}
