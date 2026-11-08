// The new-folder dialog with name validation.
import { useState, useRef, useEffect } from 'react'
import { useFocusTrap } from './hooks/use-focus-trap'
import { BBButton } from '../design'

interface NewFolderDialogProps {
  open: boolean
  onClose: () => void
  onCreate: (name: string) => void
}

export function NewFolderDialog({ open, onClose, onCreate }: NewFolderDialogProps) {
  const [name, setName] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const focusTrapRef = useFocusTrap<HTMLFormElement>(open)

  useEffect(() => {
    if (open) {
      setName('')
      // Small delay to allow the dialog to render before focusing
      const t = setTimeout(() => inputRef.current?.focus(), 50)
      return () => clearTimeout(t)
    }
  }, [open])

  if (!open) return null

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    const trimmed = name.trim()
    if (!trimmed) return
    onCreate(trimmed)
    onClose()
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      onClick={onClose}
    >
      {/* Backdrop */}
      <div className="absolute inset-0 bg-ink/20" />

      {/* Dialog */}
      <form
        ref={focusTrapRef}
        role="dialog"
        aria-modal="true"
        aria-label="New folder"
        onSubmit={handleSubmit}
        onClick={(e) => e.stopPropagation()}
        className="relative w-full max-w-[24rem] bg-paper border border-line-2 rounded-xl shadow-3 overflow-hidden"
      >
        <div className="px-xl py-lg border-b border-line">
          <h3 className="text-sm font-semibold text-ink">New folder</h3>
        </div>

        <div className="px-xl py-lg">
          <label className="block text-xs font-medium text-ink-2 mb-1.5">
            Folder name
          </label>
          <div className="flex items-center gap-2 border rounded-md bg-paper px-3 py-2 border-line focus-within:ring-2 focus-within:ring-amber/30 focus-within:border-amber-deep">
            <input
              ref={inputRef}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Contracts"
              className="flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-4"
            />
          </div>
        </div>

        <div className="px-xl py-md border-t border-line flex justify-end gap-2">
          <BBButton type="button" variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </BBButton>
          <BBButton type="submit" variant="amber" size="sm" disabled={!name.trim()}>
            Create
          </BBButton>
        </div>
      </form>
    </div>
  )
}
