/**
 * Pure line-level diff for the conflict-resolution window (task 1546 finding
 * 2). ConflictWindow.tsx's own doc-comment previously admitted the diff body
 * was a hardcoded placeholder ("Content from this device…" / "Content from
 * other device…") for EVERY conflict, regardless of actual file content —
 * this is the real diff that replaces it, now that `conflict_content_preview`
 * (src-tauri/src/engine_bridge.rs) can supply both sides' real text.
 *
 * Classic LCS backtrack over lines. O(n*m) time AND space, so `diffLines`
 * refuses (returns `null`) above `DIFF_MAX_CELLS` rather than allocate an
 * unbounded DP table for a huge or highly-repetitive file pair — the caller
 * falls back to showing both texts in full, un-highlighted, which is still
 * real content, just without line-level highlighting.
 */

export type DiffOpType = 'add' | 'remove' | 'same'

export interface DiffOp {
  type: DiffOpType
  line: string
}

/** (lines in A + 1) * (lines in B + 1) DP cells, above which `diffLines`
 * returns `null` instead of building the table. ~1.5M cells is comfortably
 * fast and a few MB of memory for a one-off UI action, while still covering
 * the `conflict_content_preview` 256 KiB text cap for any realistic average
 * line length. */
export const DIFF_MAX_CELLS = 1_500_000

/**
 * Returns a line-level diff of `a` -> `b` (add/remove/same ops that, read in
 * order, reconstruct both `a` — same+remove lines — and `b` — same+add
 * lines), or `null` if the two line counts' product exceeds `DIFF_MAX_CELLS`.
 */
export function diffLines(a: string, b: string): DiffOp[] | null {
  const linesA = a.split('\n')
  const linesB = b.split('\n')
  const n = linesA.length
  const m = linesB.length

  if ((n + 1) * (m + 1) > DIFF_MAX_CELLS) return null

  // dp[i][j] = length of the LCS of linesA[i:] and linesB[j:].
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = linesA[i] === linesB[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1])
    }
  }

  const ops: DiffOp[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (linesA[i] === linesB[j]) {
      ops.push({ type: 'same', line: linesA[i] })
      i++
      j++
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      ops.push({ type: 'remove', line: linesA[i] })
      i++
    } else {
      ops.push({ type: 'add', line: linesB[j] })
      j++
    }
  }
  while (i < n) {
    ops.push({ type: 'remove', line: linesA[i] })
    i++
  }
  while (j < m) {
    ops.push({ type: 'add', line: linesB[j] })
    j++
  }
  return ops
}
