/**
 * Shared `react-markdown` `a` component override (task 1563 PR #103 review
 * thread) — opens links in a NEW TAB with `noopener noreferrer`, same as the
 * read-mode markdown renderer already did. Used by both markdown-preview.tsx
 * (read mode) and the editor's split-view live preview (file-editor.tsx),
 * so a link click can never navigate the CURRENT tab away — which is what
 * let a click bypass the editor's unsaved-changes guard: the draft only
 * lives in memory, and a same-tab navigation unmounts the editor with no
 * guard in the loop at all (there's nothing to intercept — the browser is
 * leaving the page, not React).
 */
import type { AnchorHTMLAttributes, ReactNode } from 'react'
import { resolveSafeMarkdownHref } from '../lib/markdown-link'

interface MarkdownSafeLinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
  children?: ReactNode
}

export function MarkdownSafeLink({ href, children, className }: MarkdownSafeLinkProps) {
  const safeHref = resolveSafeMarkdownHref(href)
  return (
    <a href={safeHref} className={className} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  )
}
