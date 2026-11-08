/**
 * Lazy CodeMirror 6 language loader (task 1563).
 *
 * Mirrors the pattern already used for Shiki grammars in
 * `text-preview.tsx` (LANG_LOADERS): each language is its own dynamic
 * import, so Vite/Rollup emits a separate chunk per language and opening a
 * `.json` file never downloads the Rust or Python grammar.
 *
 * Language ids match the strings `file-preview.tsx`'s EXT_LANGUAGE/
 * CODE_MIME_TYPES maps already produce (so the two stay in lockstep by
 * construction), plus `'markdown'` for .md/.mdx. An id with no entry here —
 * e.g. 'go', 'shell', 'toml' — degrades gracefully to no language extension:
 * the editor still gets line numbers, bracket matching, etc., just no
 * grammar-aware highlighting yet. Add a loader here (and the matching
 * `@codemirror/lang-*` package) to light one up.
 */

import type { Extension } from '@codemirror/state'

type LangLoader = () => Promise<Extension>

const LOADERS: Record<string, LangLoader> = {
  markdown: async () => {
    const [{ markdown }, { GFM }] = await Promise.all([
      import('@codemirror/lang-markdown'),
      import('@lezer/markdown'),
    ])
    return markdown({ extensions: [GFM] })
  },
  javascript: async () => (await import('@codemirror/lang-javascript')).javascript(),
  typescript: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ typescript: true }),
  jsx: async () => (await import('@codemirror/lang-javascript')).javascript({ jsx: true }),
  tsx: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ jsx: true, typescript: true }),
  json: async () => (await import('@codemirror/lang-json')).json(),
  css: async () => (await import('@codemirror/lang-css')).css(),
  html: async () => (await import('@codemirror/lang-html')).html(),
  python: async () => (await import('@codemirror/lang-python')).python(),
  rust: async () => (await import('@codemirror/lang-rust')).rust(),
  sql: async () => (await import('@codemirror/lang-sql')).sql(),
  yaml: async () => (await import('@codemirror/lang-yaml')).yaml(),
  xml: async () => (await import('@codemirror/lang-xml')).xml(),
  cpp: async () => (await import('@codemirror/lang-cpp')).cpp(),
  c: async () => (await import('@codemirror/lang-cpp')).cpp(),
}

const cache = new Map<string, Promise<Extension | null>>()

/** Loads (and caches) the CodeMirror language extension for a language id.
 *  Returns null for an unmapped id or an id of `undefined` (plain text). */
export function loadLanguageExtension(lang: string | undefined): Promise<Extension | null> {
  if (!lang) return Promise.resolve(null)
  const loader = LOADERS[lang]
  if (!loader) return Promise.resolve(null)
  let cached = cache.get(lang)
  if (!cached) {
    cached = loader().catch(() => null)
    cache.set(lang, cached)
  }
  return cached
}
