// Find in a PDF (DW-1506): every match on every page, case-insensitive. The page text comes from pdf.js's
// getTextContent (render.ts); this module only counts and locates, so the view can say "2 of 3" and jump.
export interface PdfMatch {
  /** 0-based page index. */
  page: number
}

/** Every occurrence of `query` in `pages`, in page order. Empty or blank queries match nothing. */
export function findMatches(pages: string[], query: string): PdfMatch[] {
  const needle = query.trim().toLowerCase()
  if (!needle) return []
  const matches: PdfMatch[] = []
  pages.forEach((text, page) => {
    const haystack = text.toLowerCase()
    let at = haystack.indexOf(needle)
    while (at !== -1) {
      matches.push({ page })
      at = haystack.indexOf(needle, at + needle.length)
    }
  })
  return matches
}

/** The page the current match sits on, or 0 when there is no match. */
export const matchPage = (matches: PdfMatch[], current: number) => matches[current]?.page ?? 0
