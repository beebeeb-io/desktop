// The rendered markdown preview.
import { useEffect, useState } from 'react'
import Markdown from 'react-markdown'
import { MarkdownSafeLink } from './markdown-safe-link'

interface MarkdownPreviewProps {
  blob: Blob
}

export function MarkdownPreview({ blob }: MarkdownPreviewProps) {
  const [text, setText] = useState<string | null>(null)

  useEffect(() => {
    blob.text().then(setText)
  }, [blob])

  if (text === null) return null

  return (
    <div
      className="w-full max-w-[620px] overflow-auto rounded-md bg-paper text-ink"
      style={{
        maxHeight: '92%',
        boxShadow: '0 10px 40px rgba(0,0,0,0.2)',
        padding: '42px 56px 48px',
      }}
    >
      <Markdown
        components={{
          h1: ({ children }) => (
            <h1 className="mb-2 text-[22px] font-bold leading-tight tracking-tight text-ink">
              {children}
            </h1>
          ),
          h2: ({ children }) => (
            <h2 className="mb-2 mt-7 text-base font-semibold text-ink">
              {children}
            </h2>
          ),
          h3: ({ children }) => (
            <h3 className="mb-1.5 mt-5 text-sm font-semibold text-ink">
              {children}
            </h3>
          ),
          p: ({ children }) => (
            <p className="mb-3 text-[13px] leading-[1.7] text-ink-2">
              {children}
            </p>
          ),
          ul: ({ children }) => (
            <ul className="mb-3 list-disc pl-5 text-[13px] leading-[1.7] text-ink-2">
              {children}
            </ul>
          ),
          ol: ({ children }) => (
            <ol className="mb-3 list-decimal pl-5 text-[13px] leading-[1.7] text-ink-2">
              {children}
            </ol>
          ),
          li: ({ children }) => <li className="mb-1">{children}</li>,
          code: ({ className, children }) => {
            const isBlock = className?.includes('language-')
            if (isBlock) {
              return (
                <code className="block overflow-x-auto rounded bg-paper-2 p-3 font-mono text-[11px] leading-relaxed text-ink-2">
                  {children}
                </code>
              )
            }
            return (
              <code className="rounded bg-paper-2 px-1 py-0.5 font-mono text-[11px] text-ink-2">
                {children}
              </code>
            )
          },
          pre: ({ children }) => (
            <pre className="mb-3 overflow-x-auto rounded bg-paper-2 p-3">
              {children}
            </pre>
          ),
          blockquote: ({ children }) => (
            <blockquote className="mb-3 border-l-2 border-line-2 pl-4 text-ink-3">
              {children}
            </blockquote>
          ),
          hr: () => <hr className="my-6 border-line" />,
          a: ({ href, children }) => (
            <MarkdownSafeLink href={href} className="text-amber-deep underline underline-offset-2">
              {children}
            </MarkdownSafeLink>
          ),
        }}
      >
        {text}
      </Markdown>
    </div>
  )
}
