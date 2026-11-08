/**
 * CodeMirror 6 React wrapper for the Beebeeb text editor (task 1563).
 *
 * A thin, imperative-core wrapper: the `EditorView` is created once and
 * mutated via `dispatch`/`Compartment.reconfigure`, not recreated on every
 * React render — recreating it would drop undo history and cursor position
 * on every keystroke's parent re-render.
 *
 * Wired in: line numbers, fold gutter, bracket matching + auto-closing +
 * colourisation (bracket-colors.ts), indentation guides
 * (@replit/codemirror-indentation-markers), selection-match highlighting,
 * search/replace panel (⌘F / ⌘⌥F), undo/redo, auto-indent, soft-wrap
 * (toggle), the app's ⌘S save binding (preventDefault so the browser's own
 * "Save page" dialog never fires), and the brand theme (codemirror-theme.ts).
 * The language extension AND the minimap both load lazily (editor-langs.ts,
 * minimap-extension.ts) — everything else here is small enough to ship in
 * the main editor chunk.
 */

import { useEffect, useImperativeHandle, useRef, forwardRef } from 'react'
import { EditorState, Compartment, type Extension } from '@codemirror/state'
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  dropCursor,
  rectangularSelection,
} from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  indentUnit,
} from '@codemirror/language'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { search, searchKeymap, highlightSelectionMatches } from '@codemirror/search'
import { indentationMarkers } from '@replit/codemirror-indentation-markers'
import { bbEditorExtensions } from './codemirror-theme'
import { loadLanguageExtension } from './editor-langs'
import { loadMinimapExtension } from './minimap-extension'
import { bracketColorization } from './bracket-colors'
import './editor.css'

export interface CursorPosition {
  line: number
  col: number
}

export interface CodeMirrorEditorHandle {
  focus: () => void
  getDoc: () => string
}

interface CodeMirrorEditorProps {
  /** Initial document text. Only read on mount — updates go through
   *  `onChange`, not prop diffing, so an external re-render never clobbers
   *  the user's in-progress edit. */
  initialDoc: string
  /** Our internal language id (editor-langs.ts), e.g. 'markdown', 'typescript'. */
  language?: string
  wrap: boolean
  readOnly?: boolean
  onChange: (doc: string) => void
  onCursorChange: (pos: CursorPosition) => void
  onSave: () => void
  className?: string
  ariaLabel?: string
}

export const CodeMirrorEditor = forwardRef<CodeMirrorEditorHandle, CodeMirrorEditorProps>(
  function CodeMirrorEditor(
    { initialDoc, language, wrap, readOnly = false, onChange, onCursorChange, onSave, className, ariaLabel },
    ref,
  ) {
    const containerRef = useRef<HTMLDivElement | null>(null)
    const viewRef = useRef<EditorView | null>(null)
    const languageCompartment = useRef(new Compartment())
    const wrapCompartment = useRef(new Compartment())
    const readOnlyCompartment = useRef(new Compartment())
    const minimapCompartment = useRef(new Compartment())
    // Latest-callback refs so the keymap/updateListener (bound once at
    // EditorView creation) always call the current closure, not a stale one
    // captured at mount time.
    const onChangeRef = useRef(onChange)
    const onCursorChangeRef = useRef(onCursorChange)
    const onSaveRef = useRef(onSave)
    onChangeRef.current = onChange
    onCursorChangeRef.current = onCursorChange
    onSaveRef.current = onSave

    useImperativeHandle(ref, () => ({
      focus: () => viewRef.current?.focus(),
      getDoc: () => viewRef.current?.state.doc.toString() ?? '',
    }))

    // Create the view once.
    useEffect(() => {
      if (!containerRef.current) return

      const saveKeymap = keymap.of([
        {
          key: 'Mod-s',
          run: () => {
            onSaveRef.current()
            return true
          },
          preventDefault: true,
        },
      ])

      const updateListener = EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          onChangeRef.current(update.state.doc.toString())
        }
        if (update.docChanged || update.selectionSet) {
          const pos = update.state.selection.main.head
          const line = update.state.doc.lineAt(pos)
          onCursorChangeRef.current({ line: line.number, col: pos - line.from + 1 })
        }
      })

      const extensions: Extension[] = [
        lineNumbers(),
        foldGutter(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        history(),
        drawSelection(),
        dropCursor(),
        rectangularSelection(),
        bracketMatching(),
        closeBrackets(),
        bracketColorization,
        indentOnInput(),
        indentUnit.of('  '),
        // Both light/dark fields point at the SAME `--ed-indent-guide*` custom
        // property (editor.css) rather than using this package's own
        // light/dark switch (which keys off CodeMirror's `EditorView.darkTheme`
        // extension — not set here, since theming already runs entirely on the
        // app's own CSS variables toggled by `.dark` on <html>).
        indentationMarkers({
          highlightActiveBlock: true,
          colors: {
            light: 'var(--ed-indent-guide)',
            dark: 'var(--ed-indent-guide)',
            activeLight: 'var(--ed-indent-guide-active)',
            activeDark: 'var(--ed-indent-guide-active)',
          },
        }),
        highlightSelectionMatches(),
        search({ top: true }),
        wrapCompartment.current.of(wrap ? EditorView.lineWrapping : []),
        languageCompartment.current.of([]),
        readOnlyCompartment.current.of(EditorState.readOnly.of(readOnly)),
        minimapCompartment.current.of([]),
        keymap.of([
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...searchKeymap,
          ...historyKeymap,
          ...foldKeymap,
          indentWithTab,
        ]),
        saveKeymap,
        updateListener,
        ...bbEditorExtensions,
      ]

      const state = EditorState.create({
        doc: initialDoc,
        extensions,
      })

      const view = new EditorView({
        state,
        parent: containerRef.current,
      })
      viewRef.current = view

      // Initial cursor position report (Ln 1, Col 1).
      onCursorChangeRef.current({ line: 1, col: 1 })

      return () => {
        view.destroy()
        viewRef.current = null
      }
      // Intentionally mount-once: initialDoc/readOnly/language changes after
      // mount are applied via the effects below through Compartments, not by
      // recreating the view (which would wipe undo history).
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])

    // Lazily load + apply the minimap once (task 1563 design screen 02) —
    // not per-language, so this doesn't belong in the language-reconfigure
    // effect below, and not part of the main bundle (minimap-extension.ts).
    useEffect(() => {
      let cancelled = false
      loadMinimapExtension().then((ext) => {
        if (cancelled) return
        const view = viewRef.current
        if (!view) return
        view.dispatch({ effects: minimapCompartment.current.reconfigure(ext) })
      })
      return () => {
        cancelled = true
      }
    }, [])

    // Load + apply the language extension whenever `language` changes.
    useEffect(() => {
      let cancelled = false
      loadLanguageExtension(language).then((ext) => {
        if (cancelled) return
        const view = viewRef.current
        if (!view) return
        view.dispatch({
          effects: languageCompartment.current.reconfigure(ext ?? []),
        })
      })
      return () => {
        cancelled = true
      }
    }, [language])

    // Apply wrap toggle.
    useEffect(() => {
      const view = viewRef.current
      if (!view) return
      view.dispatch({
        effects: wrapCompartment.current.reconfigure(wrap ? EditorView.lineWrapping : []),
      })
    }, [wrap])

    // Apply readOnly toggle.
    useEffect(() => {
      const view = viewRef.current
      if (!view) return
      view.dispatch({
        effects: readOnlyCompartment.current.reconfigure(EditorState.readOnly.of(readOnly)),
      })
    }, [readOnly])

    return (
      <div
        ref={containerRef}
        className={`bb-editor ${className ?? ''}`}
        role="textbox"
        aria-label={ariaLabel ?? 'Code editor'}
        aria-multiline="true"
      />
    )
  },
)
