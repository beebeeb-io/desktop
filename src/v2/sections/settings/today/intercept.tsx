// B12's General and Notifications link to pages B13 now owns; this routes those links inside the settings view
// (capture phase, so the anchor never navigates) and leaves B12's panes untouched.
import { createElement, type ComponentType, type ReactNode } from 'react'
import { General } from '../shell/general'
import { Notifications } from '../shell/notifications'
import { useSettings, type PaneId } from '../shell/context'

function Wrap({ children, routes }: { children?: ReactNode; routes: Record<string, PaneId> }) {
  const { go } = useSettings()
  return (
    <div className="contents" onClickCapture={(e) => {
      const a = (e.target as HTMLElement).closest('a')
      const href = a?.getAttribute('href') ?? ''
      const hit = Object.keys(routes).find((k) => href.includes(k))
      if (hit) { e.preventDefault(); e.stopPropagation(); go(routes[hit]) }
    }}>{children}</div>
  )
}

const wrap = (el: ComponentType<object>, routes: Record<string, PaneId>): ComponentType<object> => (props: object) => createElement(Wrap, { routes }, createElement(el, props))
export const InterceptLinks = {
  general: wrap(General as ComponentType, { 'page=diagnostics': 'diagnostics' }),
  notifications: wrap(Notifications as ComponentType, { 'page=recent': 'recent' }),
}
