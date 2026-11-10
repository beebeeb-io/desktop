// DW-1901 / DW-1902: the NSIS installer (Windows draws it; this is the board's reference drawing). The pages and words
// are the template's own; Beebeeb supplies its name and icon (src-tauri/tauri.conf.json > bundle.windows.nsis).
import { BBProductIcon } from '@beebeeb/shared/design'

export function InstallerScene({ page }: { page: 'installing' | 'finish' }) {
  const btn = (label: string, dim = false, primary = false) => (
    <span data-installer-button={label} className={`inline-block min-w-[74px] rounded-[3px] border px-3 py-[3px] text-center text-[12px] ${dim ? 'border-win-button-line-3 text-win-ink-dim' : primary ? 'border-win-accent text-win-ink-strong' : 'border-win-button-line-2 text-win-ink-strong'} bg-win-surface-2`}>{label}</span>
  )
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-win-desktop font-sans" data-desktop>
      <div className="grid min-h-0 flex-1 place-items-center">
        <div role="dialog" aria-label="Beebeeb Setup" className="w-[500px] border border-win-line-strong bg-win-surface-2 text-win-ink-strong shadow-2xl" data-system-drawn>
          <div className="flex h-8 items-center gap-2 bg-white px-2 text-[12px]"><BBProductIcon product="drive" size={14} />Beebeeb Setup<span className="ml-auto text-win-ink-3">✕</span></div>
          {page === 'installing' ? (
            <>
              <div className="flex items-start justify-between border-b border-win-line-2 bg-white px-4 py-3">
                <div><div className="text-[12px] font-semibold">Installing</div><div className="pl-3 text-[12px]">Please wait while Beebeeb is being installed.</div></div>
                <BBProductIcon product="drive" size={32} />
              </div>
              <div className="px-6 py-5 text-[12px]">
                <div className="mb-2">Extract: Beebeeb.exe</div>
                <div className="h-4 border border-win-progress-track bg-white p-px" role="progressbar" aria-valuenow={62} aria-label="Installing"><div className="h-full w-[62%] bg-win-progress-fill" /></div>
                <div className="mt-4">{btn('Show details')}</div>
              </div>
              <div className="h-16" />
            </>
          ) : (
            <div className="flex">
              <div className="grid w-[164px] place-items-center bg-win-panel py-16"><BBProductIcon product="drive" size={56} /></div>
              <div className="flex-1 bg-white px-5 py-4 text-[12px]">
                <div className="mb-3 text-[15px] font-semibold">Completing Beebeeb Setup</div>
                <p className="mb-2">Beebeeb has been installed on your computer.</p>
                <p className="mb-5">Click Finish to close Setup.</p>
                <label className="flex items-center gap-2"><input type="checkbox" checked readOnly aria-label="Run Beebeeb" />Run Beebeeb</label>
                <div className="h-16" />
              </div>
            </div>
          )}
          <div className="flex justify-end gap-2 border-t border-win-line-2 px-3 py-2.5">
            {page === 'installing' ? <>{btn('< Back', true)}{btn('Next >', true)}{btn('Cancel')}</> : <>{btn('< Back', true)}{btn('Finish', false, true)}{btn('Cancel', true)}</>}
          </div>
        </div>
      </div>
      <div className="flex h-12 flex-none items-center justify-center gap-2 bg-win-taskbar" data-taskbar-row>
        {['Start', 'Search', 'File Explorer', 'Edge'].map((t) => <span key={t} className="h-7 w-7 rounded bg-win-taskbar-app" title={t} aria-hidden />)}
        {page === 'installing' && <span className="h-7 w-7 rounded bg-win-taskbar-app-open" aria-hidden />}
      </div>
    </div>
  )
}
