// A small code colourer on the design tokens only: strings green, numbers red, comments faint, keywords bold.
import { Fragment } from 'react'

const KEYWORDS = new Set(['import', 'from', 'def', 'class', 'return', 'if', 'else', 'elif', 'for', 'in', 'with', 'as', 'while', 'try', 'except', 'finally', 'raise', 'and', 'or', 'not', 'is', 'None', 'True', 'False', 'const', 'let', 'var', 'function', 'export', 'fn', 'pub', 'use', 'struct', 'impl', 'match', 'async', 'await', 'lambda', 'pass', 'yield'])
const TOKEN = /(#.*$|\/\/.*$)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(\b\d+(?:\.\d+)?\b)|([A-Za-z_][A-Za-z0-9_]*)/g

export function HighlightedLine({ line }: { line: string }) {
  const out: React.ReactNode[] = []
  let last = 0
  let prevWord = ''
  for (const m of line.matchAll(TOKEN)) {
    const at = m.index ?? 0
    if (at > last) out.push(line.slice(last, at))
    const [text, comment, str, num, word] = m
    if (comment) out.push(<span key={at} data-tok="comment" className="text-ink-4">{text}</span>)
    else if (str) out.push(<span key={at} data-tok="string" className="text-green-text">{text}</span>)
    else if (num) out.push(<span key={at} data-tok="number" className="text-red-text">{text}</span>)
    else if (word && KEYWORDS.has(word)) { out.push(<b key={at} data-tok="keyword" className="font-semibold text-ink">{text}</b>); prevWord = word }
    else if (word) out.push(<span key={at} data-tok="name" className={prevWord === 'def' || prevWord === 'class' ? 'font-semibold text-ink' : 'text-ink-2'}>{text}</span>)
    if (!word || !KEYWORDS.has(word)) prevWord = ''
    last = at + text.length
  }
  if (last < line.length) out.push(line.slice(last))
  return <Fragment>{out.length ? out : '​'}</Fragment>
}
