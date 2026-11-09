// Text and Markdown editing (DW-1520..1527): the editor on the shared CodeMirror, the save dialogs, the New file
// dialogs. A save is a new version through the source's saveText; a changed remote is a conflict, never an overwrite.
import { useRef, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { CodeMirrorEditor, type CodeMirrorEditorHandle } from '@beebeeb/shared/drive-ui'
import { EditorView } from '@codemirror/view'
import { openSearchPanel } from '@codemirror/search'
import { BTN, BTN_AMBER, FOCUS, Modal } from '../files/ui'
import { editorLanguage, saveNoteText, type SaveNote } from './model'
import { focusOnMount } from './Chrome'

export function TextEditor({ name, initialDoc, wrap = false, onChange, onSave, onCursor }: { name: string; initialDoc: string; wrap?: boolean; onChange: (doc: string) => void; onSave: () => void; onCursor?: (line: number, col: number) => void }) {
  const ref = useRef<CodeMirrorEditorHandle>(null)
  return (
    <div data-viewer="editor" className="min-h-0 flex-1 overflow-hidden bg-paper">
      <CodeMirrorEditor
        ref={ref} initialDoc={initialDoc} language={editorLanguage(name)} wrap={wrap} onChange={onChange} onSave={onSave}
        onCursorChange={(p) => onCursor?.(p.line, p.col)} ariaLabel={`Editing ${name}`} className="h-full"
      />
    </div>
  )
}

/** Open the editor's own find panel (CodeMirror's, which the shared editor's search keymap opens on Ctrl+F inside the
 *  text): from anywhere in the window, Ctrl+F finds in the text (DW-1523). */
export function openEditorFind(root: HTMLElement | null): boolean {
  const cm = root?.querySelector<HTMLElement>('.cm-editor')
  const view = cm ? EditorView.findFromDOM(cm) : null
  return view ? openSearchPanel(view) : false
}

/** "Changes not saved yet · version 3": the toolbar's meta in each state. */
export const editMeta = (dirty: boolean, version: number, saving?: boolean) =>
  saving ? 'Saving…' : dirty ? `Changes not saved yet · version ${version}` : `Saved · version ${version}`

export function SaveButton({ onClick, disabled }: { onClick?: () => void; disabled?: boolean }) {
  return (
    <button type="button" onClick={onClick} disabled={disabled} className={`${BTN} gap-1.5 px-3 disabled:opacity-40`}>
      <Icon name="lock" size={13} />Save<span className="ml-1 font-mono text-[11px] font-normal text-ink-3">Ctrl+S</span>
    </button>
  )
}

/** The saved message under the toolbar (DW-1524). A save is "Saved on this PC · encrypting…" until the engine has
 *  uploaded it, then "Saved · encrypted · version N" with Rust's number (the lead's ruling on #273 finding 12).
 *  `version` alone is the board's drawing of the encrypted state. */
export function SavedMessage({ version, note, onVersions }: { version?: number; note?: SaveNote; onVersions?: () => void }) {
  const n: SaveNote = note ?? { kind: 'encrypted', version: version ?? 1 }
  return (
    <div role="status" data-saved={n.kind} className="flex h-10 flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 text-[13px] text-ink">
      <span className="text-ink-2"><Icon name={n.kind === 'local' ? 'upload' : 'lock'} size={13} /></span>
      <span className="flex-1">{saveNoteText(n)}</span>
      <a href="#versions" onClick={(e) => { e.preventDefault(); onVersions?.() }} className="text-[13px] text-ink underline">Versions</a>
    </div>
  )
}

export function SavedStatus({ version, time }: { version: number; time: string }) {
  return <footer role="status" className="flex h-7 flex-none items-center border-t border-line bg-paper-2 px-3 text-[12px] text-ink-3">Saved {'·'} encrypted {'·'} version {version} {'·'} {time}</footer>
}

/** Leaving with changes asks first (DW-1525). Also on Ctrl+W, quitting and going to another file. */
export function UnsavedAlert({ name, onSave, onDiscard, onKeep }: { name: string; onSave?: () => void; onDiscard?: () => void; onKeep?: () => void }) {
  return (
    <div role="presentation" className="fixed inset-0 z-50 grid place-items-center bg-ink/25">
      <div role="alertdialog" aria-modal="true" aria-label="You have unsaved changes" aria-describedby="unsaved-body" className="w-[420px] rounded-xl border border-line bg-paper p-5 shadow-2xl">
        <h2 className="m-0 mb-1 text-[15px] font-semibold text-ink">You have unsaved changes</h2>
        <p id="unsaved-body" className="m-0 mb-4 text-[13px] leading-5 text-ink-2">Your edits to {name} aren{'’'}t saved. Save them as a new, encrypted version, or discard them.</p>
        <div className="flex items-center gap-2">
          <button type="button" className={BTN_AMBER} onClick={onSave} ref={focusOnMount}>Save</button>
          <button type="button" className={`${BTN} text-red-text`} onClick={onDiscard}>Discard changes</button>
          <span className="flex-1" />
          <button type="button" className={BTN} onClick={onKeep}>Keep editing</button>
        </div>
      </div>
    </div>
  )
}

export interface Side { label: string; when: string; note: string; preview: string }

/** The Two versions dialog while editing (DW-1526; as DW-0429). Nothing is overwritten; the other side stays in history. */
export function EditConflictDialog({ name, mine, theirs, changedOn, onKeep, onCancel }: {
  name: string; mine: Side; theirs: Side; changedOn: string; onKeep?: (c: 'mine' | 'theirs' | 'both') => void; onCancel?: () => void
}) {
  const card = (s: Side) => (
    <div data-side className="flex min-w-0 flex-1 flex-col gap-2 rounded-lg border border-line bg-paper p-2">
      <pre className="m-0 h-32 overflow-hidden whitespace-pre-wrap rounded-md bg-paper-2 p-2 font-mono text-[11px] leading-4 text-ink-2">{s.preview}</pre>
      <div className="px-1 pb-1"><div className="text-[13px] font-medium text-ink">{s.label}</div><div className="font-mono text-[11px] text-ink-3">{s.when} {'·'} {s.note}</div></div>
    </div>
  )
  return (
    <Modal
      title="Two versions" label="Two versions" width={680} onClose={onCancel}
      footer={<>
        <button type="button" className={BTN} onClick={onCancel}>Cancel</button><span className="flex-1" />
        <button type="button" className={BTN} onClick={() => onKeep?.('mine')}>Keep this PC{'’'}s</button>
        <button type="button" className={BTN} onClick={() => onKeep?.('theirs')}>Keep {theirs.label.split(' ·')[0]}{'’'}s</button>
        <button type="button" ref={focusOnMount} className={BTN_AMBER} onClick={() => onKeep?.('both')}>Keep both</button>
      </>}
    >
      <p className="m-0 mb-1 text-ink"><span className="font-medium">{name}</span></p>
      <p className="m-0 mb-3 text-ink-3">{changedOn}</p>
      <div className="flex gap-3">{card(mine)}{card(theirs)}</div>
    </Modal>
  )
}

// ---- New file: pick a type (DW-1521), name it (DW-1522) ----
export interface FileType { id: 'md' | 'txt' | 'docx' | 'xlsx' | 'pptx'; label: string; ext: string; office: boolean }
export const FILE_TYPES: FileType[] = [
  { id: 'md', label: 'Markdown', ext: '.md', office: false },
  { id: 'txt', label: 'Text', ext: '.txt', office: false },
  { id: 'docx', label: 'Word', ext: '.docx', office: true },
  { id: 'xlsx', label: 'Excel', ext: '.xlsx', office: true },
  { id: 'pptx', label: 'PowerPoint', ext: '.pptx', office: true },
]

export function NewFileTypeDialog({ selected, onSelect, onContinue, onCancel }: { selected: FileType['id']; onSelect?: (id: FileType['id']) => void; onContinue?: () => void; onCancel?: () => void }) {
  return (
    <Modal
      title="New file" label="New file" width={440} onClose={onCancel}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onCancel}>Cancel</button><button type="button" className={BTN_AMBER} onClick={onContinue}>Continue</button></>}
    >
      <p className="m-0 mb-3 text-ink-3">Encrypted on this PC before it leaves</p>
      <div role="radiogroup" aria-label="Pick a type" className="flex flex-col gap-1">
        {FILE_TYPES.map((t) => (
          <button key={t.id} type="button" role="radio" aria-checked={t.id === selected} data-type={t.id} onClick={() => onSelect?.(t.id)} onDoubleClick={onContinue}
            className={`flex h-11 items-center gap-3 rounded-lg border px-3 text-left text-[13px] text-ink ${t.id === selected ? 'border-ink' : 'border-line hover:bg-paper-2'} ${FOCUS}`}>
            <span className="text-ink-2"><Icon name="file-text" size={18} /></span>
            <span className="flex-1 font-medium">{t.label}</span>
            <span className="font-mono text-[12px] text-ink-3">{t.ext}</span>
          </button>
        ))}
      </div>
    </Modal>
  )
}

const SUGGESTIONS = ['.txt', '.py', '.js', '.json', '.sh', '.log', '.csv']
export const NAME_RULE = /[\\/:*?"<>|]/

/** The same name rules as Rename: not empty, none of \ / : * ? " < > |, at most 255 characters. */
export function nameError(name: string, taken: boolean): string | null {
  if (name.trim() === '') return 'Give it a name.'
  if (NAME_RULE.test(name)) return 'A name can’t contain \\ / : * ? " < > |'
  if (name.length > 255) return 'That name is too long.'
  if (taken) return 'There’s already a file with that name here.'
  return null
}

export function NewFileNameDialog({ markdown, name, ext, error, onName, onExt, onCreate, onCancel }: { markdown?: boolean; name: string; ext: string; error?: string | null; onName?: (v: string) => void; onExt?: (v: string) => void; onCreate?: () => void; onCancel?: () => void }): ReactNode {
  const field = `h-9 rounded-md border bg-paper px-3 text-[13px] text-ink ${error ? 'border-red-border' : 'border-line'} ${FOCUS}`
  return (
    <Modal
      title={markdown ? 'New Markdown file' : 'New text file'} label={markdown ? 'New Markdown file' : 'New text file'} width={440} onClose={onCancel}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onCancel}>Cancel</button><button type="button" className={BTN_AMBER} onClick={onCreate}>Create file</button></>}
    >
      <form onSubmit={(e) => { e.preventDefault(); onCreate?.() }} className="flex flex-col gap-3">
        <div className="flex gap-2">
          <label className="flex min-w-0 flex-1 flex-col gap-1 text-[12px] text-ink-3">Name
            <input aria-label="Name" value={name} onChange={(e) => onName?.(e.target.value)} ref={focusOnMount} onFocus={(e) => e.currentTarget.select()} spellCheck={false} className={field} />
          </label>
          <label className="flex w-24 flex-col gap-1 text-[12px] text-ink-3">Extension
            <input aria-label="Extension" value={ext} onChange={(e) => onExt?.(e.target.value)} spellCheck={false} className={`${field} font-mono`} />
          </label>
        </div>
        {error && <p role="alert" className="m-0 text-[12px] text-red-text">{error}</p>}
        {!markdown && (
          <div className="flex flex-wrap gap-1.5" aria-label="Suggestions">
            {SUGGESTIONS.map((s) => <button key={s} type="button" onClick={() => onExt?.(s)} className={`h-7 rounded-md border border-line px-2 font-mono text-[12px] text-ink-2 hover:bg-paper-2 ${FOCUS}`}>{s}</button>)}
          </div>
        )}
      </form>
    </Modal>
  )
}
