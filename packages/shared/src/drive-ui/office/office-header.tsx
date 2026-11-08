/**
 * OfficeHeader (task 1567) — breadcrumb + unsaved dot, ⌘K search pill, Share,
 * amber Save ⌘S. Same chrome language as the mockup's `.chromebar` and the
 * text editor's save button (file-editor.tsx's `editor-save`) — Save is
 * amber + the lock glyph + the ⌘S hint, disabled while there's nothing to
 * save.
 */

import { Icon, BBButton } from '../../design'

export interface OfficeHeaderProps {
  breadcrumb: string[]
  filename: string
  dirty: boolean
  saving: boolean
  onBack: () => void
  onOpenPalette: () => void
  onShare?: () => void
  onSave: () => void
  /** SF-0002: repaired-from-iOS files cannot be written in place. */
  readOnly?: boolean
}

export function OfficeHeader({ breadcrumb, filename, dirty, saving, onBack, onOpenPalette, onShare, onSave, readOnly }: OfficeHeaderProps) {
  return (
    <div className="flex h-[52px] shrink-0 items-center gap-3.5 border-b border-line bg-paper-2 px-4" data-testid="office-header">
      <button
        type="button"
        onClick={onBack}
        aria-label="Back to preview"
        data-testid="office-back"
        className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-3 hover:bg-paper-3 hover:text-ink"
      >
        <Icon name="x" size={14} />
      </button>
      <div className="flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[13.5px] text-ink-3">
        {breadcrumb.map((crumb, i) => (
          <span key={i} className="flex items-center gap-1.5">
            {i > 0 && <span className="text-ink-4">›</span>}
            <span>{crumb}</span>
          </span>
        ))}
        <span className="text-ink-4">›</span>
        <b className="truncate font-semibold text-ink" title={filename}>
          {filename}
        </b>
        {dirty && (
          <span
            className="ml-0.5 h-[7px] w-[7px] shrink-0 rounded-full bg-amber shadow-[0_0_0_3px_var(--color-amber-bg)]"
            title="Unsaved changes"
            data-testid="office-unsaved-dot"
          />
        )}
      </div>
      <div className="flex-1" />
      <button
        type="button"
        onClick={onOpenPalette}
        data-testid="office-open-palette"
        className="flex h-8 items-center gap-2 rounded-full border border-line bg-paper-3 px-3 text-[13px] text-ink-3"
      >
        <Icon name="search" size={13} />
        <span className="hidden sm:inline">Search commands, styles, files…</span>
        <kbd className="rounded border border-line bg-paper px-1.5 py-0.5 font-mono text-[10px] text-ink-4">⌘K</kbd>
      </button>
      {onShare && (
        <button
          type="button"
          onClick={onShare}
          data-testid="office-share"
          className="flex h-8 items-center gap-1.5 rounded-md border border-line px-3 text-[13px] font-medium text-ink hover:bg-paper-3"
        >
          <Icon name="share" size={14} />
          Share
        </button>
      )}
      <BBButton size="sm" variant="amber" onClick={onSave} disabled={saving || !dirty || readOnly} data-testid="office-save">
        <Icon name="lock" size={12} className="mr-1" />
        {saving ? 'Saving…' : 'Save'}
        <span className="ml-1.5 font-mono text-[10px] opacity-70">⌘S</span>
      </BBButton>
    </div>
  )
}
