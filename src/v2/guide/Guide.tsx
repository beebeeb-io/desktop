// The first-time guide (DW-0380, DW-0301 to DW-0305). Each tip sits over the real window, which stays usable; the
// target is found in the DOM and outlined. Esc is Skip; Skip ends the whole guide. Runs once, right after setup;
// Settings › General › Show the guide again runs it again (`startGuide`). ./GuideHost.tsx runs it in the live window.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { TaskbarStrip } from '../chrome/TrayArt'

export type TipId = 'new' | 'file-menu' | 'shared' | 'photos' | 'settings'

export const TIPS: { id: TipId; text: string }[] = [
  { id: 'new', text: 'Add files and photos here. They’re encrypted before they upload. You can also drop files from File Explorer.' },
  { id: 'file-menu', text: 'Right-click a file to share it, move it or keep it on this PC.' },
  { id: 'shared', text: 'Files and folders you’ve shared, or that others shared with you.' },
  { id: 'photos', text: 'Photos your iPhone and iPad backed up. New ones arrive on their own.' },
  { id: 'settings', text: 'Storage, devices, backups and app lock. Ctrl+, opens them, and you can see this guide again there.' },
]

/** The first file in the list or the grid (a name with an extension): what "Right-click a file" points at. None when the
 *  window shows no file (only folders, or nothing): the guide then skips that tip (DW-0302). */
function firstFileRow(): Element | null {
  const rows = Array.from(document.querySelectorAll('main [role="row"][data-row], main [role="row"][data-tile]'))
  // A list row's first cell is its name; a tile's name sits in a cell's title (its first cell is the picture).
  const name = (r: Element) => Array.from(r.querySelectorAll('[role="gridcell"]'))
    .map((c) => (c.querySelector('[title]')?.getAttribute('title') ?? c.textContent ?? '').trim()).find(Boolean) ?? ''
  return rows.find((r) => /\.[a-z0-9]{1,5}$/i.test(name(r))) ?? null
}

/** Where each tip points: found by what the person sees, so the guide follows the real window. */
export function findTarget(id: TipId): Element | null {
  const buttons = Array.from(document.querySelectorAll('button, a'))
  const byText = (t: string) => buttons.find((b) => b.textContent?.trim() === t) ?? null
  switch (id) {
    case 'new': return byText('New')
    case 'file-menu': return firstFileRow()
    case 'shared': return byText('Shared') ?? document.querySelector('nav [href*="shared"]')
    case 'photos': return byText('Photos') ?? document.querySelector('nav [href*="photos"]')
    case 'settings': return document.querySelector('[aria-label^="Settings"]')
  }
}

interface Box { top: number; left: number; width: number; height: number }

/** Where the card goes: under the outlined target, or above it when it would not fit under it (the gear, DW-0305). */
export function cardPlace(box: Box | null, cardHeight: number, view: { width: number; height: number }): { top: number; left: number } {
  if (!box) return { top: 120, left: 260 }
  const left = Math.min(Math.max(box.left, 12), view.width - 332)
  const below = box.top + box.height + 10
  if (below + cardHeight <= view.height - 8) return { top: below, left }
  return { top: Math.max(8, box.top - 10 - cardHeight), left }
}

export function GuideTip({ index, onNext, onSkip }: { index: number; onNext: () => void; onSkip: () => void }) {
  const tip = TIPS[index]
  const [box, setBox] = useState<Box | null>(null)
  const card = useRef<HTMLDivElement>(null)
  const [cardHeight, setCardHeight] = useState(140)
  // Measured when the tip opens, again for a target that draws a moment later (a section still loading), and on resize.
  useLayoutEffect(() => {
    let tries = 0
    let timer = 0
    const measure = () => {
      if (card.current) setCardHeight(card.current.offsetHeight)
      const el = findTarget(tip.id)
      if (!el) {
        setBox(null)
        if (tries++ < 20) timer = window.setTimeout(measure, 100)
        return
      }
      const r = el.getBoundingClientRect()
      const left = Math.max(2, r.left - 4)
      setBox({ top: r.top - 4, left, width: Math.min(r.width + 8, window.innerWidth - left - 2), height: r.height + 8 })
    }
    measure()
    window.addEventListener('resize', measure)
    return () => { window.clearTimeout(timer); window.removeEventListener('resize', measure) }
  }, [tip.id])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onSkip() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onSkip])
  const last = index === TIPS.length - 1
  const place = cardPlace(box, cardHeight, { width: window.innerWidth, height: window.innerHeight })
  return (
    <div className="pointer-events-none absolute inset-0 z-40" data-guide-tip={tip.id} data-screen={`guide-${tip.id}`}>
      {box && <div data-outline className="absolute rounded-md border-2 border-ink ring-4 ring-paper/60" style={box} />}
      <div role="dialog" aria-label={`Tip ${index + 1} of ${TIPS.length}`} aria-describedby={`guide-tip-${tip.id}`} ref={card} className="pointer-events-auto absolute w-[320px] rounded-lg border border-line bg-paper p-4 shadow-xl" style={place}>
        <p id={`guide-tip-${tip.id}`} className="m-0 text-[13px] leading-[19px] text-ink">{tip.text}</p>
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

/** DW-0380's tip: where Beebeeb lives when the window is closed. `className` places it (over the taskbar on the board). */
export function TrayTipCard({ onGotIt, className = '' }: { onGotIt: () => void; className?: string }) {
  return (
    <div role="dialog" aria-label="Beebeeb lives in the notification area" aria-describedby="guide-tray-tip" className={`w-[300px] rounded-lg border border-line bg-paper p-4 shadow-xl ${className}`} data-tray-tip data-screen="guide-tray">
      <div className="text-[13px] font-semibold text-ink">Beebeeb lives in the notification area</div>
      <p id="guide-tray-tip" className="mb-3 mt-1 text-[12px] leading-[17px] text-ink-2">Click the b to see what’s syncing. Closing this window keeps Beebeeb running.</p>
      {/* eslint-disable-next-line jsx-a11y/no-autofocus */}
      <button type="button" autoFocus onClick={onGotIt} className="h-7 rounded-md border border-line bg-paper-2 px-3 text-[12px] font-medium text-ink"><span className="text-[12px]">Got it</span></button>
      <span className="absolute -bottom-1.5 right-[64px] h-3 w-3 rotate-45 border-b border-r border-line bg-paper" aria-hidden />
    </div>
  )
}

/** DW-0380: the desktop, the window on Files, the taskbar's notification area, and the tip under the b. Once per PC. */
export function TrayTipScene({ window: win, onGotIt }: { window: ReactNode; onGotIt: () => void }) {
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-win-desktop font-sans" data-desktop>
      <div className="relative min-h-0 flex-1 p-6 pb-3">
        <div className="relative h-full overflow-hidden rounded-lg shadow-2xl [&>div]:!h-full">{win}</div>
      </div>
      <div className="relative flex h-12 flex-none items-center justify-end bg-win-taskbar px-3" data-taskbar-row>
        <TaskbarStrip state="up-to-date" mode="light" />
        <TrayTipCard onGotIt={onGotIt} className="absolute bottom-[54px] right-3" />
      </div>
    </div>
  )
}

const DUE_KEY = 'bb.v2.guide.due'
/** Sent on the window when the guide is asked for (Settings › General › Show the guide again, in this window). */
export const GUIDE_EVENT = 'bb:v2-guide'
/** This window asked for it: the guide still starts here when storage refuses (the setup window's ask is then lost). */
let askedHere = false

/** Whether the guide was asked for and has not run to its end or been skipped since. */
export function guideDue(): boolean {
  if (askedHere) return true
  try { return localStorage.getItem(DUE_KEY) === '1' } catch { return false }
}

/** Ask for the guide: setup's last step (the main window opens next) and Settings › General › Show the guide again. */
export function startGuide(): void {
  askedHere = true
  try { localStorage.setItem(DUE_KEY, '1') } catch { /* storage refused: this window still starts it (askedHere) */ }
  window.dispatchEvent(new Event(GUIDE_EVENT))
}

/** Skip or Done: the guide does not come back by itself. */
export function markGuideSeen(): void {
  askedHere = false
  try { localStorage.removeItem(DUE_KEY) } catch { /* storage refused: nothing was stored */ }
}
