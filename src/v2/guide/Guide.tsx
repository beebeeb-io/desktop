// The first-time guide (DW-0380, DW-0301 to DW-0305). Each tip sits over the real window, which stays usable; the
// target is found in the DOM and outlined. Esc is Skip; Skip ends the whole guide. Runs once, right after setup;
// Settings › General › Show the guide again runs it again.
import { useEffect, useLayoutEffect, useState, type ReactNode } from 'react'
import { TaskbarStrip } from '../chrome/TrayArt'

export type TipId = 'new' | 'file-menu' | 'shared' | 'photos' | 'settings'

export const TIPS: { id: TipId; text: string }[] = [
  { id: 'new', text: 'Add files and photos here. They’re encrypted before they upload. You can also drop files from File Explorer.' },
  { id: 'file-menu', text: 'Right-click a file to share it, move it or keep it on this PC.' },
  { id: 'shared', text: 'Files and folders you’ve shared, or that others shared with you.' },
  { id: 'photos', text: 'Photos your iPhone and iPad backed up. New ones arrive on their own.' },
  { id: 'settings', text: 'Storage, devices, backups and app lock. Ctrl+, opens them, and you can see this guide again there.' },
]

/** Where each tip points: found by what the person sees, so the guide follows the real window. */
function findTarget(id: TipId): Element | null {
  const buttons = Array.from(document.querySelectorAll('button, a'))
  const byText = (t: string) => buttons.find((b) => b.textContent?.trim() === t) ?? null
  switch (id) {
    case 'new': return byText('New')
    case 'file-menu': return Array.from(document.querySelectorAll('main [role="row"], main li, main tr')).find((r) => r.textContent?.includes('Contract draft.pdf')) ?? null
    case 'shared': return byText('Shared') ?? document.querySelector('nav [href*="shared"]')
    case 'photos': return byText('Photos') ?? document.querySelector('nav [href*="photos"]')
    case 'settings': return document.querySelector('[aria-label^="Settings"]')
  }
}

interface Box { top: number; left: number; width: number; height: number }

export function GuideTip({ index, onNext, onSkip }: { index: number; onNext: () => void; onSkip: () => void }) {
  const tip = TIPS[index]
  const [box, setBox] = useState<Box | null>(null)
  useLayoutEffect(() => {
    const el = findTarget(tip.id)
    if (!el) { setBox(null); return }
    const r = el.getBoundingClientRect()
    const left = Math.max(2, r.left - 4)
    setBox({ top: r.top - 4, left, width: Math.min(r.width + 8, window.innerWidth - left - 2), height: r.height + 8 })
  }, [tip.id])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onSkip() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onSkip])
  const last = index === TIPS.length - 1
  const cardTop = box ? Math.min(box.top + box.height + 10, window.innerHeight - 170) : 120
  const cardLeft = box ? Math.min(Math.max(box.left, 12), window.innerWidth - 332) : 260
  return (
    <div className="pointer-events-none absolute inset-0 z-40" data-guide-tip={tip.id}>
      {box && <div data-outline className="absolute rounded-md border-2 border-ink shadow-[0_0_0_4px_oklch(1_0_0/0.6)]" style={box} />}
      <div role="dialog" aria-label={`Tip ${index + 1} of ${TIPS.length}`} className="pointer-events-auto absolute w-[320px] rounded-lg border border-line bg-paper p-4 shadow-xl" style={{ top: cardTop, left: cardLeft }}>
        <p className="m-0 text-[13px] leading-[19px] text-ink">{tip.text}</p>
        <div className="mt-3 flex items-center justify-between">
          <span className="font-mono text-[11px] text-ink-3" data-tip-count>{index + 1} of {TIPS.length}</span>
          <div className="flex gap-2">
            {!last && <button type="button" onClick={onSkip} className="h-7 rounded-md px-2.5 text-[12px] text-ink-2 hover:bg-paper-2"><span className="text-[12px]">Skip</span></button>}
            {/* eslint-disable-next-line jsx-a11y/no-autofocus */}
            <button type="button" autoFocus onClick={onNext} className="h-7 rounded-md border border-line bg-paper-2 px-3 text-[12px] font-medium text-ink hover:bg-paper-3"><span className="text-[12px]">{last ? 'Done' : 'Next'}</span></button>
          </div>
        </div>
      </div>
    </div>
  )
}

/** DW-0380: the desktop, the window on Files, the taskbar's notification area, and the tip under the b. Once per PC. */
export function TrayTipScene({ window: win, onGotIt }: { window: ReactNode; onGotIt: () => void }) {
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-[oklch(0.62_0.03_230)] font-sans" data-desktop>
      <div className="relative min-h-0 flex-1 p-6 pb-3">
        <div className="relative h-full overflow-hidden rounded-lg shadow-2xl [&>div]:!h-full">{win}</div>
      </div>
      <div className="relative flex h-12 flex-none items-center justify-end bg-[oklch(0.95_0_0/0.92)] px-3" data-taskbar-row>
        <TaskbarStrip state="up-to-date" mode="light" />
        <div role="dialog" aria-label="Beebeeb lives in the notification area" className="absolute bottom-[54px] right-3 w-[300px] rounded-lg border border-line bg-paper p-4 shadow-xl" data-tray-tip>
          <div className="text-[13px] font-semibold text-ink">Beebeeb lives in the notification area</div>
          <p className="mb-3 mt-1 text-[12px] leading-[17px] text-ink-2">Click the b to see what’s syncing. Closing this window keeps Beebeeb running.</p>
          {/* eslint-disable-next-line jsx-a11y/no-autofocus */}
          <button type="button" autoFocus onClick={onGotIt} className="h-7 rounded-md border border-line bg-paper-2 px-3 text-[12px] font-medium text-ink"><span className="text-[12px]">Got it</span></button>
          <span className="absolute -bottom-1.5 right-[64px] h-3 w-3 rotate-45 border-b border-r border-line bg-paper" aria-hidden />
        </div>
      </div>
    </div>
  )
}

const SEEN_KEY = 'bb.v2.guide.seen'

/** Whether the guide still has to run on this PC (Settings › General › Show the guide again clears it). */
export function guideDue(): boolean {
  try { return localStorage.getItem(SEEN_KEY) !== '1' } catch { return false }
}

export function markGuideSeen(): void {
  try { localStorage.setItem(SEEN_KEY, '1') } catch { /* private mode: the guide may show again */ }
}

export function showGuideAgain(): void {
  try { localStorage.removeItem(SEEN_KEY) } catch { /* nothing to clear */ }
}
