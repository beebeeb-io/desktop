/**
 * Sum/Average/Count of a Calc selection, computed from clipboard TEXT (task
 * 1567, Calc lane).
 *
 * Pure parsing only -- no DOM, no bbOffice -- so it's independently
 * unit-testable. See `./use-calc-selection-stats.ts` for why clipboard text
 * is the only real signal available (bbOffice has no cell/range-value read
 * op, and onSelectionChange never fires for a Calc range in this engine
 * build -- both verified against the real artifact, not assumed).
 *
 * `.uno:Copy` puts the selection on the clipboard as plain text, cells
 * separated by `\t` (columns) and `\n`/`\r\n` (rows) -- confirmed against the
 * real engine with a real 3-cell numeric range (clipboard read back as
 * "10\n20\n30\n"). Cells can carry the SAME display formatting the sheet
 * shows (e.g. "€21,400", matching design/office-editor-shots/calc-*.png),
 * so this parses that formatting rather than assuming plain numbers.
 */

export interface CalcSelectionStats {
  sum: number
  average: number
  count: number
  /** True when at least one parsed cell carried a currency symbol -- the
   *  caller uses this to decide whether to render the stats with a currency
   *  prefix or as plain numbers. */
  hadCurrency: boolean
}

const CURRENCY_SYMBOLS = /[€$£¥]/g

/**
 * Parses one clipboard cell's text into a number, honoring the same display
 * conventions LibreOffice itself renders: a currency symbol, thousands
 * commas, a trailing "%" (divided by 100, matching spreadsheet convention),
 * and parenthesised negatives (the common "(123)" accounting notation).
 * Returns null for anything that isn't a recognizable number (blank cells,
 * text labels like "Category" or "Engineering") -- those are excluded from
 * the stats, not treated as zero.
 */
function parseNumericToken(raw: string): { value: number; hadCurrency: boolean } | null {
  let s = raw.trim()
  if (!s) return null
  const hadCurrency = CURRENCY_SYMBOLS.test(s)
  CURRENCY_SYMBOLS.lastIndex = 0

  let negative = false
  if (/^\(.*\)$/.test(s)) {
    negative = true
    s = s.slice(1, -1)
  }
  s = s.replace(CURRENCY_SYMBOLS, '').replace(/[+\s]/g, '')

  let percent = false
  if (s.endsWith('%')) {
    percent = true
    s = s.slice(0, -1)
  }

  // Leading minus survives the currency/percent strip (e.g. "-€800" or "−€800",
  // the design mockup's own minus-sign glyph U+2212).
  if (s.startsWith('-') || s.startsWith('−')) {
    negative = negative || true
    s = s.slice(1)
  }

  if (!s) return null
  s = s.replace(/,/g, '') // thousands grouping, matching the app's own locale convention (€21,400)
  if (!/^\d+(\.\d+)?$/.test(s)) return null

  let value = Number(s)
  if (Number.isNaN(value)) return null
  if (percent) value = value / 100
  if (negative) value = -Math.abs(value)
  return { value, hadCurrency }
}

/**
 * Computes Sum/Average/Count over every numeric cell in a clipboard-text
 * grid. Returns null when nothing in the selection parses as a number (an
 * empty selection, or a purely-text one like a header row) -- the caller
 * shows no stats rather than a misleading "Sum 0".
 */
export function parseClipboardStats(text: string): CalcSelectionStats | null {
  if (!text) return null
  const tokens = text
    .split(/[\t\r\n]+/)
    .map((t) => t.trim())
    .filter((t) => t.length > 0)

  const numbers: number[] = []
  let hadCurrency = false
  for (const token of tokens) {
    const parsed = parseNumericToken(token)
    if (!parsed) continue
    numbers.push(parsed.value)
    if (parsed.hadCurrency) hadCurrency = true
  }

  if (numbers.length === 0) return null
  const sum = numbers.reduce((a, b) => a + b, 0)
  return { sum, average: sum / numbers.length, count: numbers.length, hadCurrency }
}

/** "€45,200" / "€11,300" -- rounded to whole units, matching the mockup's own
 *  currency display (no decimals shown for Sum/Average in the status bar). */
export function formatCalcCurrency(n: number): string {
  const rounded = Math.round(n)
  const sign = rounded < 0 ? '-' : ''
  return `${sign}€${Math.abs(rounded).toLocaleString('en-US')}`
}

/** Plain-number formatting for a non-currency selection, e.g. "1,234.5". */
export function formatCalcNumber(n: number): string {
  return n.toLocaleString('en-US', { maximumFractionDigits: 2 })
}
