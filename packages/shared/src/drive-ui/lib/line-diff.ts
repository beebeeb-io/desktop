/**
 * Minimal line-level diff for the editor's conflict dialog (task 1563).
 *
 * Classic LCS-based diff — good enough for a "what changed" view on
 * reasonably sized text files. Bounded by MAX_DIFF_LINES so a pathological
 * input (near the 2 MB editable ceiling, thousands of one-line changes)
 * can't blow up the O(n*m) DP table's memory; callers get `null` back and
 * show a "too large to diff" fallback instead.
 */

export type DiffLineType = 'same' | 'add' | 'del'

export interface DiffLine {
  type: DiffLineType
  text: string
}

/** ~1500 lines each side keeps the DP table under ~10 MB (Uint32Array). */
export const MAX_DIFF_LINES = 1500

export function diffLines(a: string, b: string): DiffLine[] | null {
  const linesA = a.split('\n')
  const linesB = b.split('\n')
  const n = linesA.length
  const m = linesB.length
  if (n > MAX_DIFF_LINES || m > MAX_DIFF_LINES) return null

  const dp: Uint32Array[] = new Array(n + 1)
  for (let i = 0; i <= n; i++) dp[i] = new Uint32Array(m + 1)
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] =
        linesA[i] === linesB[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1])
    }
  }

  const result: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (linesA[i] === linesB[j]) {
      result.push({ type: 'same', text: linesA[i] })
      i++
      j++
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      result.push({ type: 'del', text: linesA[i] })
      i++
    } else {
      result.push({ type: 'add', text: linesB[j] })
      j++
    }
  }
  while (i < n) {
    result.push({ type: 'del', text: linesA[i] })
    i++
  }
  while (j < m) {
    result.push({ type: 'add', text: linesB[j] })
    j++
  }
  return result
}

/** True when the diff contains at least one add/del line. */
export function hasChanges(diff: DiffLine[]): boolean {
  return diff.some((l) => l.type !== 'same')
}

export type SideCellType = 'same' | 'del' | 'add' | 'empty'

export interface SideCell {
  type: SideCellType
  text: string
  /** 1-based line number on THIS side, or undefined for an 'empty' filler
   *  cell (a row that exists only to align the OTHER side's add/del line). */
  lineNo?: number
}

export interface SideBySideRow {
  left: SideCell
  right: SideCell
}

/**
 * Converts a unified diff (line-diff's own `diffLines` output: a flat
 * same/add/del sequence) into a SIDE-BY-SIDE row layout — left = the
 * earlier/"theirs" text, right = the later/"ours" text — matching the
 * conflict dialog's design (design/editor-1563.html screen 04: two columns,
 * each with its own line numbers and add/remove markers, task 1563 PR #103
 * review — the shipped dialog was unified, not side-by-side).
 *
 * A 'same' line becomes one row with matching content on both sides. A
 * 'del' line (only in the LEFT/earlier text) becomes a row with content on
 * the left and an 'empty' filler on the right, so the two columns stay
 * vertically aligned row-for-row — the same convention GitHub/GitLab's split
 * diff view uses for a plain line-level diff (no move detection).
 */
export function toSideBySideRows(diff: DiffLine[]): SideBySideRow[] {
  const rows: SideBySideRow[] = []
  let leftLine = 0
  let rightLine = 0
  for (const line of diff) {
    if (line.type === 'same') {
      leftLine++
      rightLine++
      rows.push({
        left: { type: 'same', text: line.text, lineNo: leftLine },
        right: { type: 'same', text: line.text, lineNo: rightLine },
      })
    } else if (line.type === 'del') {
      leftLine++
      rows.push({
        left: { type: 'del', text: line.text, lineNo: leftLine },
        right: { type: 'empty', text: '' },
      })
    } else {
      rightLine++
      rows.push({
        left: { type: 'empty', text: '' },
        right: { type: 'add', text: line.text, lineNo: rightLine },
      })
    }
  }
  return rows
}
