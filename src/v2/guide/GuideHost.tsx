// The first-time guide in the live window (DW-0380, DW-0301 to DW-0305). It runs when it was asked for (setup's last
// step, Settings › General › Show the guide again: ./Guide.tsx `startGuide`), over the real window, which stays usable.
// Step 0 is the notification-area tip; the window cannot draw over the taskbar, so the tip sits in the window's corner
// nearest to it (bottom right) with its arrow pointing there. Steps 1 to 5 are the five tips; the third and fourth switch
// the window to Shared and Photos by themselves, the last returns to Files. Skip (or Esc) and Done end the guide.
import { useCallback, useEffect, useRef, useState } from 'react'
import type { Navigation, SectionId } from '../routes'
import { seed } from '../seed'
import { findTarget, GUIDE_EVENT, GuideTip, guideDue, markGuideSeen, TIPS, TrayTipCard } from './Guide'

/** The section each step shows: 0 the tray tip over Files, 1 to 5 the tips (TIPS order). */
export const GUIDE_SECTIONS: readonly SectionId[] = ['files', 'files', 'files', 'shared', 'photos', 'files']
export const GUIDE_LAST = TIPS.length

/** The step after `step`, or null when the guide ends. The file-menu tip is skipped when the list has no file (DW-0302). */
export function nextGuideStep(step: number, hasFile: boolean): number | null {
  const next = step + 1
  if (next > GUIDE_LAST) return null
  if (TIPS[next - 1]?.id === 'file-menu' && !hasFile) return nextGuideStep(next, hasFile)
  return next
}

export function GuideHost({ nav }: { nav: Navigation }) {
  const [step, setStep] = useState<number | null>(() => seed<number | null>('guide.step', null))
  const { go, route } = nav
  const at = useRef({ step, section: route.section })
  useEffect(() => { at.current = { step, section: route.section } }, [step, route.section])

  // Each step shows its own section ("Switches to the real Shared view by itself"); the person may move on from there.
  const show = useCallback((n: number | null) => {
    setStep(n)
    const want = n == null ? null : GUIDE_SECTIONS[n]
    if (want && at.current.section !== want) go({ section: want })
  }, [go])

  // Asked for before this window opened (setup), while it is open (Show the guide again), or from the setup window.
  useEffect(() => {
    const start = () => { if (guideDue() && at.current.step == null) show(0) }
    start()
    window.addEventListener(GUIDE_EVENT, start)
    window.addEventListener('storage', start)
    window.addEventListener('focus', start)
    return () => {
      window.removeEventListener(GUIDE_EVENT, start)
      window.removeEventListener('storage', start)
      window.removeEventListener('focus', start)
    }
  }, [show])

  const end = (toFiles: boolean) => {
    markGuideSeen()
    show(null)
    if (toFiles && at.current.section !== 'files') go({ section: 'files' })
  }
  const next = () => { if (step != null) show(nextGuideStep(step, findTarget('file-menu') != null)) }

  if (step == null) return null
  if (step === 0) {
    return (
      <div className="pointer-events-none absolute inset-0 z-40" data-guide-tip="tray">
        <TrayTipCard onGotIt={next} className="pointer-events-auto absolute bottom-10 right-4" />
      </div>
    )
  }
  // Done on the last tip ends the guide on Files (DW-0305).
  const last = step === GUIDE_LAST
  return <GuideTip key={step} index={step - 1} onNext={last ? () => end(true) : next} onSkip={() => end(false)} />
}
