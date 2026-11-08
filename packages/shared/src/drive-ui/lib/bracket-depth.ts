/**
 * Bracket-pair colourisation — pure depth computation (task 1563 design
 * screen 02: "bracket-pair colours"). Pairs each opening bracket with its
 * matching closer via a stack and assigns both the SAME 0-based nesting
 * depth, so a caller can cycle a small palette (`depth % paletteSize`) and
 * have every bracket in a pair render the same colour — the standard
 * "rainbow brackets" behavior.
 *
 * No DOM, no CodeMirror — plain string in, unit-testable in isolation. The
 * CodeMirror wiring (bracket-colors.ts) calls this once per document change
 * and renders decorations only for the currently visible range.
 *
 * Deliberately naive about strings/comments (a `)` inside a string literal
 * is still treated as a real bracket) — matching the common lightweight
 * "rainbow brackets" implementations. A syntax-tree-aware version would need
 * a full per-language token classifier; given this only affects a cosmetic
 * nesting-depth colour cycle (never editing behaviour, never bracket
 * matching/auto-close, both of which already use CodeMirror's own
 * tree-aware `bracketMatching()`/`closeBrackets()`), the simplification is
 * intentional.
 */

const OPENERS = new Set(['(', '{', '['])
const CLOSER_TO_OPENER: Record<string, string> = { ')': '(', '}': '{', ']': '[' }

export interface BracketMark {
  /** 0-based character offset into the scanned text. */
  index: number
  char: string
  /** 0-based nesting depth shared by an opener and its matching closer. */
  depth: number
}

/**
 * Scans are skipped above this length (~1 MB of characters) — bracket
 * colourisation is a cosmetic affordance, not a correctness feature, and a
 * full-document rescan on every keystroke of the largest editable files
 * (task 1563's 2 MB ceiling) would cost more than it's worth. The editor
 * still works fully without it; brackets just render in the plain
 * punctuation colour.
 */
export const MAX_BRACKET_SCAN_LENGTH = 1_000_000

export function computeBracketDepths(text: string): BracketMark[] {
  if (text.length > MAX_BRACKET_SCAN_LENGTH) return []
  const marks: BracketMark[] = []
  const stack: string[] = []
  let depth = 0
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]
    if (OPENERS.has(ch)) {
      stack.push(ch)
      marks.push({ index: i, char: ch, depth })
      depth++
    } else {
      const wants = CLOSER_TO_OPENER[ch]
      if (wants && stack[stack.length - 1] === wants) {
        stack.pop()
        depth--
        marks.push({ index: i, char: ch, depth })
      }
      // An unmatched closer (no opener on the stack, or the wrong type) is
      // left undecorated — there's no pair to colour it as part of.
    }
  }
  return marks
}
