// The rename dialog for a file or folder.
import { useState, useRef, useEffect } from 'react'
import { useFocusTrap } from './hooks/use-focus-trap'
import { BBButton } from '../design'
import { Icon } from '../design'

interface RenameDialogProps {
  open: boolean
  onClose: () => void
  currentName: string
  onRename: (newName: string) => void
  /** Lowercase decrypted names of sibling files already in this folder
   *  (excluding the file being renamed). Task 1544 finding 3: renaming into
   *  one of these would create a same-name duplicate that a later re-upload
   *  could silently auto-version onto the wrong file — so it's blocked here
   *  with an explicit message instead. */
  existingNames?: Set<string>
}

export function RenameDialog({ open, onClose, currentName, onRename, existingNames }: RenameDialogProps) {
  const [name, setName] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const focusTrapRef = useFocusTrap<HTMLFormElement>(open)

  useEffect(() => {
    if (open) {
      setName(currentName)
      const t = setTimeout(() => {
        if (inputRef.current) {
          inputRef.current.focus()
          // Select everything before the last extension
          const dotIdx = currentName.lastIndexOf('.')
          inputRef.current.setSelectionRange(0, dotIdx > 0 ? dotIdx : currentName.length)
        }
      }, 50)
      return () => clearTimeout(t)
    }
  }, [open, currentName])

  if (!open) return null

  const trimmed = name.trim()
  const isDuplicate =
    trimmed !== '' && trimmed !== currentName && (existingNames?.has(trimmed.toLowerCase()) ?? false)

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!trimmed || trimmed === currentName || isDuplicate) return
    onRename(trimmed)
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
        aria-label="Rename"
        onSubmit={handleSubmit}
        onClick={(e) => e.stopPropagation()}
        className="relative w-full max-w-[24rem] bg-paper border border-line-2 rounded-xl shadow-3 overflow-hidden"
      >
        <div className="px-xl py-lg border-b border-line flex items-center gap-2">
          <Icon name="file" size={13} className="text-ink-2" />
          <h3 className="text-sm font-semibold text-ink">Rename</h3>
        </div>

        <div className="px-xl py-lg">
          <label className="block text-xs font-medium text-ink-2 mb-1.5">
            New name
          </label>
          <div
            className={`flex items-center gap-2 border rounded-md bg-paper px-3 py-2 focus-within:ring-2 ${
              isDuplicate
                ? 'border-red focus-within:ring-red/30 focus-within:border-red'
                : 'border-line focus-within:ring-amber/30 focus-within:border-amber-deep'
            }`}
          >
            <input
              ref={inputRef}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Enter new name"
              aria-invalid={isDuplicate}
              className="flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-4"
            />
          </div>
          {isDuplicate && (
            <p className="mt-1.5 text-xs text-red">
              A file named &quot;{trimmed}&quot; already exists here.
            </p>
          )}
        </div>

        <div className="px-xl py-md border-t border-line flex justify-end gap-2">
          <BBButton type="button" variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </BBButton>
          <BBButton
            type="submit"
            variant="amber"
            size="sm"
            disabled={!trimmed || trimmed === currentName || isDuplicate}
          >
            Rename
          </BBButton>
        </div>
      </form>
    </div>
  )
}
