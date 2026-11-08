// DW-1201's picture: the Windows taskbar (Windows draws it) with the b pressed, the flyout 12 px above it and 12 px
// from the right edge, and the Beebeeb Drive window behind it on Files.
import type { ReactNode } from 'react'
import { V2Shell } from '../V2App'

const TASKBAR = 48

function Tray({ children }: { children?: ReactNode }) {
  return <div className="flex items-center gap-1 pr-3 text-[11px] text-ink-2">{children}</div>
}

export function DesktopWithFlyout({ flyout }: { flyout: ReactNode }) {
  return (
    <div className="relative h-screen w-screen overflow-hidden bg-paper-3 font-sans">
      <div className="absolute inset-x-0 top-0 overflow-hidden [&>div]:h-full!" style={{ bottom: TASKBAR }} data-backdrop="window">
        <V2Shell />
      </div>
      <div className="absolute inset-x-0 bottom-0 flex items-center justify-between border-t border-line bg-paper-2 px-3" style={{ height: TASKBAR }} role="toolbar" aria-label="Taskbar">
        <span />
        <div className="flex items-center gap-2" aria-label="Open apps">
          <span className="grid size-8 place-items-center rounded-md bg-paper-3 font-mono text-[11px] text-ink" data-taskbar-app="beebeeb">b</span>
          <span className="grid size-8 place-items-center rounded-md text-ink-3" aria-hidden="true">E</span>
        </div>
        <Tray>
          <button type="button" aria-label="Beebeeb, up to date" aria-pressed="true" data-notification-b="pressed" className="grid size-8 place-items-center rounded-md bg-paper-3 font-semibold text-ink">b</button>
          <span className="px-1 font-mono">10:42</span>
        </Tray>
      </div>
      <div className="absolute" style={{ right: 12, bottom: TASKBAR + 12 }}>{flyout}</div>
    </div>
  )
}
