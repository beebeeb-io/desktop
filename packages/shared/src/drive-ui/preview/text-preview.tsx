// The plain-text preview with size guarding.
import { useEffect, useState, useRef, useCallback } from 'react'
import { Icon } from '../../design'
import type {
  HighlighterCore,
  LanguageRegistration,
  ThemeRegistrationRaw,
} from '@shikijs/types'

interface TextPreviewProps {
  blob: Blob
  /** Optional language hint for syntax coloring (e.g. 'rust', 'typescript') */
  language?: string
  /** Optional filename for the header badge */
  filename?: string
}

// ─── Shiki token types ────────────────────────────────────────────────────────

interface ShikiToken {
  content: string
  color?: string
  fontStyle?: number // bit flags: 1=italic, 2=bold, 4=underline
}

interface ShikiResult {
  lines: ShikiToken[][]
  bg: string
  fg: string
}

const SHIKI_THEME = 'github-dark'
const MAX_LINES = 500

// ─── Lazy grammar registry ────────────────────────────────────────────────────
// Each entry is a function that returns a dynamic import for ONE TextMate
// grammar. Splitting them as individual import calls lets Vite/Rollup emit a
// separate chunk per language, so previewing a `.json` never downloads the
// 600 kB C++ grammar (the audit's P6 finding).

type LangImport = () => Promise<{ default: LanguageRegistration[] }>

const LANG_LOADERS: Record<string, LangImport> = {
  // Web / scripting
  javascript: () => import('@shikijs/langs/javascript'),
  typescript: () => import('@shikijs/langs/typescript'),
  jsx: () => import('@shikijs/langs/jsx'),
  tsx: () => import('@shikijs/langs/tsx'),
  html: () => import('@shikijs/langs/html'),
  css: () => import('@shikijs/langs/css'),
  scss: () => import('@shikijs/langs/scss'),
  vue: () => import('@shikijs/langs/vue'),
  svelte: () => import('@shikijs/langs/svelte'),
  astro: () => import('@shikijs/langs/astro'),

  // Systems
  rust: () => import('@shikijs/langs/rust'),
  go: () => import('@shikijs/langs/go'),
  c: () => import('@shikijs/langs/c'),
  cpp: () => import('@shikijs/langs/cpp'),
  zig: () => import('@shikijs/langs/zig'),

  // JVM
  java: () => import('@shikijs/langs/java'),
  kotlin: () => import('@shikijs/langs/kotlin'),
  scala: () => import('@shikijs/langs/scala'),

  // .NET
  csharp: () => import('@shikijs/langs/csharp'),

  // High-level
  python: () => import('@shikijs/langs/python'),
  ruby: () => import('@shikijs/langs/ruby'),
  php: () => import('@shikijs/langs/php'),
  lua: () => import('@shikijs/langs/lua'),
  perl: () => import('@shikijs/langs/perl'),
  swift: () => import('@shikijs/langs/swift'),
  dart: () => import('@shikijs/langs/dart'),
  r: () => import('@shikijs/langs/r'),
  elixir: () => import('@shikijs/langs/elixir'),
  erlang: () => import('@shikijs/langs/erlang'),
  haskell: () => import('@shikijs/langs/haskell'),
  ocaml: () => import('@shikijs/langs/ocaml'),
  nim: () => import('@shikijs/langs/nim'),
  v: () => import('@shikijs/langs/v'),

  // Shells
  bash: () => import('@shikijs/langs/bash'),
  powershell: () => import('@shikijs/langs/powershell'),

  // Data / config / markup
  json: () => import('@shikijs/langs/json'),
  yaml: () => import('@shikijs/langs/yaml'),
  toml: () => import('@shikijs/langs/toml'),
  xml: () => import('@shikijs/langs/xml'),
  markdown: () => import('@shikijs/langs/markdown'),
  sql: () => import('@shikijs/langs/sql'),
  graphql: () => import('@shikijs/langs/graphql'),

  // DevOps
  dockerfile: () => import('@shikijs/langs/docker'),
  makefile: () => import('@shikijs/langs/make'),
  cmake: () => import('@shikijs/langs/cmake'),
  hcl: () => import('@shikijs/langs/hcl'),
}

// ─── Normalise language names to Shiki IDs ────────────────────────────────────
// Most names from file-preview.tsx match Shiki IDs but a handful need mapping.

const LANG_MAP: Record<string, string> = {
  shell: 'bash',
  docker: 'dockerfile',
  make: 'makefile',
  terraform: 'hcl',
}

function toShikiLang(lang: string): string {
  return LANG_MAP[lang] ?? lang
}

// ─── Highlighter core (lazy singleton) ────────────────────────────────────────
// We deliberately do NOT use `shiki/bundle/web`. That bundle would ship a
// ~120 kB registry of every language and theme up-front. Instead we build a
// minimal core highlighter on first use and load individual grammars on
// demand via the LANG_LOADERS table above.

let highlighterPromise: Promise<HighlighterCore> | null = null
const loadedLangs = new Set<string>()

async function getHighlighter(): Promise<HighlighterCore> {
  if (!highlighterPromise) {
    highlighterPromise = (async () => {
      const [{ createHighlighterCore }, { createOnigurumaEngine }, wasm, theme] =
        await Promise.all([
          import('shiki/core'),
          import('shiki/engine/oniguruma'),
          import('shiki/wasm'),
          import('@shikijs/themes/github-dark'),
        ])
      return createHighlighterCore({
        themes: [theme.default as ThemeRegistrationRaw],
        langs: [],
        engine: createOnigurumaEngine(wasm.default),
      })
    })()
  }
  return highlighterPromise
}

async function ensureLang(shikiLang: string): Promise<boolean> {
  const loader = LANG_LOADERS[shikiLang]
  if (!loader) return false
  if (loadedLangs.has(shikiLang)) return true
  const highlighter = await getHighlighter()
  const mod = await loader()
  await highlighter.loadLanguage(mod.default as LanguageRegistration[])
  loadedLangs.add(shikiLang)
  return true
}

// ─── Render helpers ───────────────────────────────────────────────────────────

function tokenStyle(t: ShikiToken): React.CSSProperties {
  const style: React.CSSProperties = {}
  if (t.color) style.color = t.color
  if (t.fontStyle) {
    if (t.fontStyle & 1) style.fontStyle = 'italic'
    if (t.fontStyle & 2) style.fontWeight = 'bold'
    if (t.fontStyle & 4) style.textDecoration = 'underline'
  }
  return style
}

// ─── Language display label ───────────────────────────────────────────────────

const LANG_LABEL: Record<string, string> = {
  typescript: 'TypeScript', javascript: 'JavaScript', tsx: 'TSX', jsx: 'JSX',
  rust: 'Rust', go: 'Go', python: 'Python', ruby: 'Ruby', java: 'Java',
  kotlin: 'Kotlin', swift: 'Swift', cpp: 'C++', c: 'C', php: 'PHP', csharp: 'C#',
  bash: 'Shell', shell: 'Shell', powershell: 'PowerShell',
  html: 'HTML', css: 'CSS', scss: 'SCSS', json: 'JSON', yaml: 'YAML',
  toml: 'TOML', xml: 'XML', sql: 'SQL', markdown: 'Markdown',
  dockerfile: 'Dockerfile', makefile: 'Makefile', hcl: 'HCL',
  graphql: 'GraphQL', vue: 'Vue', svelte: 'Svelte',
}

function langLabel(lang: string): string {
  return LANG_LABEL[lang] ?? LANG_LABEL[toShikiLang(lang)] ?? lang.charAt(0).toUpperCase() + lang.slice(1)
}

// ─── Component ────────────────────────────────────────────────────────────────

export function TextPreview({ blob, language, filename }: TextPreviewProps) {
  const [text, setText] = useState<string | null>(null)
  const [truncated, setTruncated] = useState(false)

  // Shiki highlighted result
  const [shiki, setShiki] = useState<ShikiResult | null>(null)
  const [shikiLoading, setShikiLoading] = useState(false)

  // UI state
  const [htmlMode, setHtmlMode] = useState<'rendered' | 'source'>('rendered')
  const [copied, setCopied] = useState(false)
  const [wrap, setWrap] = useState(false)
  const copyTimeout = useRef<ReturnType<typeof setTimeout> | null>(null)

  const isCode = !!language
  const isHtml = language === 'html'

  // ── Load text ──────────────────────────────────────────────────────────────
  useEffect(() => {
    blob.text().then((raw) => {
      const lines = raw.split('\n')
      if (lines.length > MAX_LINES) {
        setText(lines.slice(0, MAX_LINES).join('\n'))
        setTruncated(true)
      } else {
        setText(raw)
      }
    })
  }, [blob])

  // ── Load Shiki when text is ready and we have a language ──────────────────
  useEffect(() => {
    if (!language || !text) return
    const shikiLang = toShikiLang(language)
    let cancelled = false
    setShikiLoading(true)
    ;(async () => {
      const supported = await ensureLang(shikiLang)
      if (cancelled) return
      if (!supported) {
        // Unknown grammar — leave plain text. No chunk fetched.
        return
      }
      const highlighter = await getHighlighter()
      if (cancelled) return
      const result = highlighter.codeToTokens(text, {
        lang: shikiLang,
        theme: SHIKI_THEME,
      })
      if (!cancelled) {
        setShiki({
          lines: result.tokens as ShikiToken[][],
          bg: result.bg ?? '#0d1117',
          fg: result.fg ?? '#e6edf3',
        })
      }
    })()
      .catch(() => {
        // Shiki failed (network error, unsupported grammar internals) —
        // the plain-text fallback stays visible.
      })
      .finally(() => {
        if (!cancelled) setShikiLoading(false)
      })
    return () => { cancelled = true }
  }, [language, text])

  // ── Copy handler ──────────────────────────────────────────────────────────
  const handleCopy = useCallback(async () => {
    if (!text) return
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      if (copyTimeout.current) clearTimeout(copyTimeout.current)
      copyTimeout.current = setTimeout(() => setCopied(false), 2000)
    } catch {
      // Clipboard unavailable
    }
  }, [text])

  if (text === null) return null

  const lines = text.split('\n')
  const gutterWidth = String(lines.length).length

  // ── Dark code theme context ────────────────────────────────────────────────
  const bg = shiki?.bg ?? (isCode ? '#0d1117' : undefined)
  const fg = shiki?.fg ?? (isCode ? '#c9d1d9' : undefined)

  // ── Toolbar ───────────────────────────────────────────────────────────────
  const toolbar = (
    <div
      className="shrink-0 flex items-center gap-2 px-3 py-1.5 border-b"
      style={isCode ? {
        backgroundColor: bg ? `color-mix(in srgb, ${bg} 80%, #ffffff0a)` : '#161b22',
        borderColor: '#30363d',
      } : undefined}
    >
      {/* Language badge / filename */}
      <div className="flex items-center gap-1.5 flex-1 min-w-0">
        {filename && (
          <span
            className="text-[11px] font-mono truncate"
            style={isCode ? { color: '#8b949e' } : { color: 'var(--color-ink-3)' }}
          >
            {filename}
          </span>
        )}
        {language && (
          <span
            className="shrink-0 text-[10px] font-medium px-1.5 py-0.5 rounded border"
            style={isCode ? {
              color: '#58a6ff',
              backgroundColor: '#1f2937',
              borderColor: '#30363d',
            } : undefined}
          >
            {langLabel(language)}
          </span>
        )}
        {shikiLoading && (
          <span className="flex items-center gap-1 shrink-0" title="Loading highlighter…">
            <svg className="animate-spin h-3 w-3" viewBox="0 0 24 24" fill="none"
              style={{ color: '#58a6ff' }} role="img" aria-label="Loading highlighter">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            <span className="text-[10px]" style={{ color: '#8b949e' }}>Loading highlighter…</span>
          </span>
        )}
      </div>

      {/* HTML mode toggle */}
      {isHtml && (
        <div className="flex gap-0.5 p-0.5 bg-paper rounded border border-line">
          {(['rendered', 'source'] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              onClick={() => setHtmlMode(mode)}
              className={`px-2.5 py-1 text-[11px] rounded transition-colors ${
                htmlMode === mode
                  ? 'bg-paper-2 text-ink font-medium'
                  : 'text-ink-3 hover:text-ink-2'
              }`}
            >
              {mode.charAt(0).toUpperCase() + mode.slice(1)}
            </button>
          ))}
        </div>
      )}

      {/* Wrap toggle */}
      {isCode && !(isHtml && htmlMode === 'rendered') && (
        <button
          type="button"
          title={wrap ? 'Disable line wrap' : 'Enable line wrap'}
          onClick={() => setWrap(v => !v)}
          className="flex items-center gap-1 text-[11px] px-2 py-1 rounded transition-colors"
          style={wrap
            ? { color: '#58a6ff', backgroundColor: '#1f2937' }
            : { color: '#8b949e' }
          }
        >
          <Icon name="link" size={11} />
          Wrap
        </button>
      )}

      {/* Copy button */}
      {text && !(isHtml && htmlMode === 'rendered') && (
        <button
          type="button"
          title="Copy code"
          onClick={handleCopy}
          className="flex items-center gap-1 text-[11px] px-2 py-1 rounded transition-colors shrink-0"
          style={copied
            ? { color: '#3fb950', backgroundColor: '#1f2937' }
            : isCode
              ? { color: '#8b949e' }
              : { color: 'var(--color-ink-3)' }
          }
        >
          <Icon name={copied ? 'check' : 'copy'} size={11} />
          {copied ? 'Copied' : 'Copy'}
        </button>
      )}
    </div>
  )

  return (
    <div
      className="flex w-full flex-col overflow-hidden rounded-md"
      style={{
        maxWidth: 860,
        maxHeight: '92%',
        boxShadow: '0 10px 40px rgba(0,0,0,0.2)',
        backgroundColor: bg ?? 'var(--color-paper)',
        color: fg,
      }}
    >
      {toolbar}

      {/* HTML rendered view — sandboxed iframe */}
      {isHtml && htmlMode === 'rendered' ? (
        <iframe
          title="HTML preview"
          srcDoc={text}
          sandbox=""
          className="w-full bg-paper"
          style={{ minHeight: 480, height: '70vh', border: 0 }}
        />
      ) : (
        <div className="overflow-auto flex-1">
          <table className="w-full border-collapse">
            <tbody>
              {lines.map((line, i) => (
                <tr
                  key={i}
                  className="group"
                  style={{ backgroundColor: 'transparent' }}
                >
                  {/* Line number gutter */}
                  <td
                    className="select-none text-right align-top font-mono text-[11px] leading-[1.7] px-3"
                    style={{
                      minWidth: gutterWidth * 8 + 24,
                      color: isCode ? '#484f58' : 'var(--color-ink-4)',
                      borderRight: `1px solid ${isCode ? '#21262d' : 'var(--color-line)'}`,
                      userSelect: 'none',
                    }}
                  >
                    {i + 1}
                  </td>

                  {/* Code content */}
                  <td
                    className="px-4 align-top font-mono text-[12px] leading-[1.7]"
                    style={{ color: fg }}
                  >
                    {shiki ? (
                      // Shiki tokens — proper per-token color spans
                      <span
                        className={wrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre'}
                      >
                        {shiki.lines[i]?.map((token, j) => (
                          <span key={j} style={tokenStyle(token)}>
                            {token.content}
                          </span>
                        )) ?? ''}
                      </span>
                    ) : (
                      // Plain text fallback while Shiki loads (or on failure).
                      // Rendered as React children — no dangerouslySetInnerHTML.
                      <span className={wrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre'}>
                        {line || '​'}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Truncation notice */}
      {truncated && (
        <div
          className="shrink-0 px-4 py-2 text-center font-mono text-[11px]"
          style={isCode
            ? { borderTop: '1px solid #21262d', backgroundColor: '#161b22', color: '#8b949e' }
            : { borderTop: '1px solid var(--color-line)', backgroundColor: 'var(--color-paper-2)', color: 'var(--color-ink-3)' }
          }
        >
          Showing first {MAX_LINES.toLocaleString()} lines — download the file for the full version.
        </div>
      )}
    </div>
  )
}
