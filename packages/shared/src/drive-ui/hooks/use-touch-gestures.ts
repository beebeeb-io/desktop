// Touch gesture recognition (swipe/long-press) for list rows.
import { useEffect, useRef } from 'react'

interface TouchGesturesOptions {
  onSwipeLeft?: () => void
  onSwipeRight?: () => void
  onSwipeDown?: () => void
  onDoubleTap?: () => void
  /** Minimum horizontal distance to trigger a swipe, in px. Default: 50 */
  threshold?: number
}

/**
 * Attaches native touch gesture handlers to a given element ref.
 *
 * - Swipe left/right: horizontal delta >= threshold AND vertical delta < 80px
 * - Swipe down: vertical delta >= 100px AND touch started in the top 30% of the viewport
 * - Double-tap: two taps within 300 ms at roughly the same location (±30px)
 *
 * No-ops on non-touch devices (guards with `'ontouchstart' in window`).
 */
export function useTouchGestures(
  ref: React.RefObject<HTMLElement | null>,
  opts: TouchGesturesOptions,
) {
  // Keep opts in a ref so the effect doesn't need to re-run when callbacks change.
  const optsRef = useRef(opts)
  optsRef.current = opts

  useEffect(() => {
    if (!('ontouchstart' in window)) return

    const el = ref.current
    if (!el) return

    let startX = 0
    let startY = 0
    let lastTapTime = 0
    let lastTapX = 0
    let lastTapY = 0

    function onTouchStart(e: TouchEvent) {
      const t = e.touches[0]
      if (!t) return
      startX = t.clientX
      startY = t.clientY
    }

    function onTouchEnd(e: TouchEvent) {
      const t = e.changedTouches[0]
      if (!t) return

      const dx = t.clientX - startX
      const dy = t.clientY - startY
      const absDx = Math.abs(dx)
      const absDy = Math.abs(dy)
      const { threshold = 50, onSwipeLeft, onSwipeRight, onSwipeDown, onDoubleTap } = optsRef.current

      // --- Swipe detection (only if touch moved enough to not be a tap) ---
      const didMove = absDx > threshold || absDy > threshold

      if (didMove) {
        if (onSwipeLeft && dx < -threshold && absDy < 80) {
          onSwipeLeft()
          return
        }
        if (onSwipeRight && dx > threshold && absDy < 80) {
          onSwipeRight()
          return
        }
        // Swipe-down: must start in the top 30% of the viewport
        const viewportHeight = window.innerHeight
        if (onSwipeDown && dy > 100 && startY < viewportHeight * 0.3 && absDx < 80) {
          onSwipeDown()
          return
        }
      }

      // --- Double-tap detection ---
      if (!didMove && onDoubleTap) {
        const now = Date.now()
        const dtTime = now - lastTapTime
        const dtX = Math.abs(t.clientX - lastTapX)
        const dtY = Math.abs(t.clientY - lastTapY)

        if (dtTime < 300 && dtX < 30 && dtY < 30) {
          onDoubleTap()
          // Reset so a triple-tap doesn't fire twice
          lastTapTime = 0
        } else {
          lastTapTime = now
          lastTapX = t.clientX
          lastTapY = t.clientY
        }
      }
    }

    el.addEventListener('touchstart', onTouchStart, { passive: true })
    el.addEventListener('touchend', onTouchEnd, { passive: true })

    return () => {
      el.removeEventListener('touchstart', onTouchStart)
      el.removeEventListener('touchend', onTouchEnd)
    }
  }, [ref])
}
