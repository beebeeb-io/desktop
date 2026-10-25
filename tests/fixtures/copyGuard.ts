/**
 * Task 1683 slice 5 (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §10.1): no developer word in any
 * string a person can read on a Mac.
 *
 * What it scans. The modules a Mac can mount (`macRoots`: what `src/main.tsx` imports, less the three Windows-only
 * windows) and every module they import, transitively, from `src/`. Windows-only windows (`WindowsApp`, `WindowsTray`,
 * `WindowsFirstRun`) are not roots: spec rows A5 to A7 keep them off a Mac, and §10.2 #25 leaves their copy alone. What it reads is a PERSON-FACING string, found with the TypeScript parser, not a regex over text:
 *   - JSX text;
 *   - the value of EVERY string-valued JSX attribute, except the ones on a machine-name denylist (`className`, `id`,
 *     `role`, `href`, `type`, `name`, `key`, `data-*`, the SVG geometry attributes, the `aria-*` relations): a custom
 *     component's `copy="..."` or `confirmLabel="..."` is copy, and a denylist that is too short fails loud (a word list
 *     hit in a machine value is a one-line addition here), where an allowlist that is too short fails silent;
 *   - the value of an object property named like copy (`title`, `label`, `hint`, `message`, `sentence`, ...);
 *   - any other string or template with two words in it, wherever it sits (a call's argument, an array's element, a
 *     ternary's branch), except where the position says it is a machine value: an import path, a `case`, a comparison,
 *     the argument of `command(...)`, `listen(...)`, `.includes(...)`, a type.
 * Comments are never strings, so a comment may explain what the words used to be.
 *
 * What it checks them against. `DEVELOPER_WORDS` is the §10.1 list, word for word, plus `raw command names`: any
 * snake_case name that Rust registers in `generate_handler!` showing up inside a sentence.
 */
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import ts from 'typescript'
import { stripComments, stripRustTestItems } from './cityGuard'

/**
 * The windows `main.tsx` mounts only on Windows (spec rows A5 to A7: not built on a Mac; §10.2 #25 leaves their copy alone).
 * Every other module `main.tsx` imports is mounted on a Mac, or serves a window that is, so it is a root. A new window
 * component in `main.tsx` is therefore scanned until someone decides, here, that a Mac never mounts it.
 */
export const WINDOWS_ONLY = ['WindowsApp.tsx', 'WindowsTray.tsx', 'WindowsFirstRun.tsx'] as const

/** The `src/` modules `main.tsx` imports (static, relative), as file names under `src/`. */
export function mainImports(repo: string): string[] {
  const srcRoot = join(repo, 'src')
  const main = join(srcRoot, 'main.tsx')
  const ast = parse(main, readFileSync(main, 'utf8'))
  const found: string[] = []
  ast.forEachChild((node) => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) {
      const target = resolveImport(main, node.moduleSpecifier.text, srcRoot)
      if (target) found.push(relative(srcRoot, target))
    }
  })
  return found
}

/** The roots a Mac scans: what `main.tsx` imports, less the Windows-only windows. */
export function macRoots(repo: string): string[] {
  return mainImports(repo).filter((file) => !(WINDOWS_ONLY as readonly string[]).includes(file))
}

/** §10.1, first bullet. Matched case-insensitively, as whole words, inside a person-facing string. */
export const DEVELOPER_WORDS: ReadonlyArray<{ id: string; pattern: RegExp }> = [
  { id: 'daemon', pattern: /\bdaemons?\b/i },
  { id: 'hydrate', pattern: /\bhydrat(?:e|es|ed|ing|ion)\b/i },
  { id: 'engine', pattern: /\bengines?\b/i },
  { id: 'control center', pattern: /\bcontrol cent(?:er|re)\b/i },
  { id: 'file surface', pattern: /\bfile surfaces?\b/i },
  { id: 'namespace', pattern: /\bnamespaces?\b/i },
  { id: 'state DB', pattern: /\bstate[ -]?db\b/i },
  { id: 'socket', pattern: /\bsockets?\b/i },
  { id: 'telemetry', pattern: /\btelemetry\b/i },
  { id: 'heartbeat', pattern: /\bheartbeats?\b/i },
  { id: 'wired', pattern: /\bwired\b/i },
  { id: 'this build', pattern: /\bthis (?:desktop )?build\b/i },
  { id: 'Future native wiring', pattern: /\bfuture native wiring\b/i },
]

const COPY_PROPERTY = new Set(['title', 'label', 'hint', 'message', 'sentence', 'detail', 'body', 'description', 'text', 'subtitle', 'summary', 'caption', 'placeholder', 'tooltip', 'heading'])
/**
 * JSX attributes whose string value is a name, a selector, a size or a relation, never words a person reads. Everything
 * else that is a string is copy (`title`, `copy`, `footnote`, `confirmLabel`, `detail`, `body`, `aria-label`, ...), and
 * so is a name nobody has met yet.
 */
const MACHINE_ATTRIBUTES = new Set([
  'className', 'id', 'role', 'href', 'src', 'type', 'name', 'key', 'ref', 'htmlFor', 'for', 'target', 'rel', 'style', 'tabIndex',
  'value', 'min', 'max', 'step', 'autoComplete', 'inputMode', 'lang', 'dir', 'focusable', 'preserveAspectRatio',
  'viewBox', 'd', 'fill', 'stroke', 'strokeWidth', 'strokeLinecap', 'strokeLinejoin', 'xmlns', 'width', 'height', 'x', 'y', 'cx', 'cy', 'r', 'rx', 'ry', 'points', 'transform',
  'aria-labelledby', 'aria-describedby', 'aria-controls', 'aria-live', 'aria-current', 'aria-haspopup', 'aria-hidden', 'aria-checked',
  'aria-expanded', 'aria-selected', 'aria-pressed', 'aria-busy', 'aria-valuemin', 'aria-valuemax', 'aria-valuenow', 'aria-modal', 'aria-atomic',
  // The component library's own enums and test markers: `kind="alert"`, `variant="error"`, `tone="amber"`, `surface="settings-config"`.
  'kind', 'variant', 'tone', 'surface', 'size', 'mode', 'align', 'icon', 'color',
])
const isMachineAttribute = (name: string) => name.startsWith('data-') || MACHINE_ATTRIBUTES.has(name)
/** A call whose string arguments are names, selectors or patterns, never words a person reads. */
const MACHINE_CALLS = new Set([
  'command', 'invoke', 'listen', 'emit', 'emitTo', 'once', 'getElementById', 'querySelector', 'querySelectorAll', 'matchMedia',
  'addEventListener', 'removeEventListener', 'includes', 'startsWith', 'endsWith', 'match', 'split', 'replace', 'replaceAll', 'test',
  'RegExp', 'getItem', 'setItem', 'removeItem', 'assert', 'error', 'warn', 'log', 'info', 'debug', 'trace', 'desc', 'it', 'describe', 'test',
  'setAttribute', 'getAttribute', 'closest', 'matches', 'import', 'require', 'resolve',
])

export interface CopyString {
  file: string
  line: number
  text: string
  /** Where the scanner found it: `jsx-text`, `attribute:<name>`, `property:<name>`, or `sentence`. */
  how: string
}

export interface Violation extends CopyString {
  rule: string
}

/** The Tauri commands Rust registers: the names a sentence must never carry. */
export function commandNames(root: string): string[] {
  const lib = join(root, 'src-tauri/src/lib.rs')
  if (!existsSync(lib)) return []
  const source = readFileSync(lib, 'utf8')
  const start = source.indexOf('tauri::generate_handler![')
  if (start < 0) return []
  const end = source.indexOf('])', start)
  return source
    .slice(start + 'tauri::generate_handler!['.length, end)
    .split(/[,\s]+/)
    .map((name) => name.replace(/^.*::/, '').replace(/[^a-z0-9_]/gi, ''))
    .filter((name) => name.includes('_'))
}

function resolveImport(from: string, specifier: string, srcRoot: string): string | null {
  if (!specifier.startsWith('.')) return null
  const base = resolve(dirname(from), specifier)
  for (const candidate of [base, `${base}.ts`, `${base}.tsx`, join(base, 'index.ts'), join(base, 'index.tsx')]) {
    if (candidate.startsWith(srcRoot) && /\.(ts|tsx)$/.test(candidate) && existsSync(candidate)) return candidate
  }
  return null
}

function parse(file: string, source: string): ts.SourceFile {
  return ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
}

/** Every `src/` module reachable by a static import from `roots` (file names under `src/`), roots included. */
export function reachableModules(repo: string, roots: readonly string[] = macRoots(repo)): string[] {
  const srcRoot = join(repo, 'src')
  const seen = new Set<string>()
  const queue = roots.map((root) => join(srcRoot, root)).filter((path) => existsSync(path))
  while (queue.length) {
    const file = queue.pop()!
    if (seen.has(file)) continue
    seen.add(file)
    const ast = parse(file, readFileSync(file, 'utf8'))
    const visit = (node: ts.Node) => {
      if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
        const isTypeOnly = ts.isImportDeclaration(node) ? node.importClause?.isTypeOnly === true : node.isTypeOnly
        const target = isTypeOnly ? null : resolveImport(file, node.moduleSpecifier.text, srcRoot)
        if (target) queue.push(target)
      }
      ts.forEachChild(node, visit)
    }
    visit(ast)
  }
  return [...seen].map((path) => relative(srcRoot, path)).sort()
}

function calleeName(call: ts.CallExpression | ts.NewExpression): string {
  const expression = call.expression
  if (ts.isIdentifier(expression)) return expression.text
  if (ts.isPropertyAccessExpression(expression)) return expression.name.text
  return ''
}

const twoWords = /\p{L}[\p{L}\p{N}'’-]*\s+\p{L}/u

/** The person-facing strings of one module. */
export function personFacingStrings(file: string, source: string): CopyString[] {
  const ast = parse(file, source)
  const found: CopyString[] = []
  const add = (node: ts.Node, text: string, how: string) => {
    const trimmed = text.replace(/\s+/g, ' ').trim()
    if (trimmed === '') return
    found.push({ file, line: ast.getLineAndCharacterOfPosition(node.getStart(ast)).line + 1, text: trimmed, how })
  }
  const textOfLiteral = (node: ts.Node): string | null => {
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text
    if (ts.isTemplateExpression(node)) return node.head.text + node.templateSpans.map((span) => `{}${span.literal.text}`).join('')
    return null
  }
  /** A string-like expression in a copy position: `cond ? 'a' : 'b'`, `a ?? 'b'`, `(x)` hold copy too. */
  const copyIn = (node: ts.Node, how: string) => {
    const text = textOfLiteral(node)
    if (text !== null) return void add(node, text, how)
    if (ts.isConditionalExpression(node)) {
      copyIn(node.whenTrue, how)
      copyIn(node.whenFalse, how)
    } else if (ts.isParenthesizedExpression(node)) {
      copyIn(node.expression, how)
    } else if (ts.isBinaryExpression(node) && [ts.SyntaxKind.QuestionQuestionToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.PlusToken].includes(node.operatorToken.kind)) {
      copyIn(node.left, how)
      copyIn(node.right, how)
    }
  }
  /** Only strings and string-building expressions: no function, call or object inside. */
  const isStringy = (node: ts.Node): boolean => {
    if (textOfLiteral(node) !== null) return !ts.isTemplateExpression(node) || node.templateSpans.every((span) => isStringy(span.expression) || ts.isIdentifier(span.expression) || ts.isPropertyAccessExpression(span.expression))
    if (ts.isConditionalExpression(node)) return isStringy(node.whenTrue) && isStringy(node.whenFalse)
    if (ts.isParenthesizedExpression(node)) return isStringy(node.expression)
    return ts.isIdentifier(node) || ts.isPropertyAccessExpression(node)
  }
  const claimed = new Set<ts.Node>()
  /** A machine value: nothing inside it is copy, whatever it says. */
  const claimMachine = (node: ts.Node) => {
    const mark = (n: ts.Node) => {
      claimed.add(n)
      ts.forEachChild(n, mark)
    }
    mark(node)
  }
  /** Copy: recorded here, and the walk goes on into it, so a call or an array inside it is read too (one string found twice counts once). */
  const claim = (node: ts.Node, how: string) => copyIn(node, how)

  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node) || ts.isLiteralTypeNode(node) || ts.isTypeNode(node)) return
    if (ts.isJsxText(node)) {
      add(node, node.text, 'jsx-text')
    } else if (ts.isJsxAttribute(node) && node.initializer) {
      const name = ts.isIdentifier(node.name) ? node.name.text : node.name.getText(ast)
      const value = ts.isJsxExpression(node.initializer) ? node.initializer.expression : node.initializer
      if (value && isMachineAttribute(name)) {
        // A name, a selector, a size: a machine value, whatever it says. An expression that is code (an arrow function,
        // a call) is still walked: it may build copy.
        if (isStringy(value)) claimMachine(value)
      } else if (value) {
        claim(value, `attribute:${name}`)
      }
    } else if (ts.isPropertyAssignment(node) && (ts.isIdentifier(node.name) || ts.isStringLiteral(node.name)) && COPY_PROPERTY.has(node.name.text)) {
      claim(node.initializer, `property:${node.name.text}`)
    } else if (ts.isPropertySignature(node) || ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node)) {
      return
    } else if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
      if (MACHINE_CALLS.has(calleeName(node))) {
        for (const argument of node.arguments ?? []) {
          // A machine call's string arguments are names and patterns; its callbacks and objects are still code.
          if (textOfLiteral(argument) !== null) claimed.add(argument)
        }
      }
    } else if (ts.isCaseClause(node)) {
      claimed.add(node.expression)
    } else if (ts.isBinaryExpression(node) && [ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken, ts.SyntaxKind.EqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsToken, ts.SyntaxKind.InKeyword].includes(node.operatorToken.kind)) {
      if (textOfLiteral(node.left) !== null) claimed.add(node.left)
      if (textOfLiteral(node.right) !== null) claimed.add(node.right)
    } else if (ts.isElementAccessExpression(node) && textOfLiteral(node.argumentExpression) !== null) {
      claimed.add(node.argumentExpression)
    } else if ((ts.isPropertyAssignment(node) || ts.isPropertyDeclaration(node)) && ts.isStringLiteral(node.name)) {
      claimed.add(node.name)
    }
    if ((ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateExpression(node)) && !claimed.has(node)) {
      const text = textOfLiteral(node)!
      if (twoWords.test(text)) add(node, text, 'sentence')
    }
    ts.forEachChild(node, visit)
  }
  visit(ast)
  // A string can be claimed after it was visited as a plain literal (a parent is visited first, so this is rare);
  // the same text at the same line from two routes counts once.
  const seen = new Set<string>()
  return found.filter((entry) => {
    const key = `${entry.line}\u0000${entry.text}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

/** The §10.1 violations of one module's source. `commands` are the names Rust registers (see `commandNames`). */
export function violationsInSource(file: string, source: string, commands: readonly string[] = []): { strings: number; violations: Violation[] } {
  const commandPattern = commands.length ? new RegExp(`(?<![\\w.-])(?:${commands.join('|')})(?![\\w-])`) : null
  const violations: Violation[] = []
  const strings = personFacingStrings(file, source)
  for (const entry of strings) {
    for (const word of DEVELOPER_WORDS) if (word.pattern.test(entry.text)) violations.push({ ...entry, rule: word.id })
    if (commandPattern?.test(entry.text) && twoWords.test(entry.text)) violations.push({ ...entry, rule: 'raw command name' })
  }
  return { strings: strings.length, violations }
}

/** Every §10.1 violation among the person-facing strings of the modules a Mac can reach. */
export function scan(repo: string, roots: readonly string[] = macRoots(repo)): { modules: string[]; strings: number; violations: Violation[] } {
  const modules = reachableModules(repo, roots)
  const commands = commandNames(repo)
  const violations: Violation[] = []
  let strings = 0
  for (const module of modules) {
    const found = violationsInSource(module, readFileSync(join(repo, 'src', module), 'utf8'), commands)
    strings += found.strings
    violations.push(...found.violations)
  }
  return { modules, strings, violations }
}

/** A string another slice still owns: pardoned for exactly `count` occurrences in `file`, and not one more. */
export interface Pardon {
  file: string
  text: string
  owner: string
  count: number
}

const occurrence = (v: { file: string; line: number; text: string }) => `${v.file}\u0000${v.line}\u0000${v.text}`

/**
 * What the pardons leave. A pardon is for one string in one file, `count` times: a second occurrence of a pardoned string
 * is `excess` and stays a violation (a pardon cannot be spent on a new string by repeating an old one), and a pardon whose
 * string has fewer occurrences than it claims, or none, is `stale` (the owner removed it: delete the entry).
 */
export function pardon(violations: readonly Violation[], pardons: readonly Pardon[]): { open: Violation[]; stale: string[] } {
  const seenOccurrences = new Map<string, Set<string>>()
  for (const v of violations) {
    const key = `${v.file}\u0000${v.text}`
    if (!seenOccurrences.has(key)) seenOccurrences.set(key, new Set())
    seenOccurrences.get(key)!.add(occurrence(v))
  }
  const allowance = new Map<string, number>()
  const stale: string[] = []
  for (const entry of pardons) {
    const key = `${entry.file}\u0000${entry.text}`
    allowance.set(key, (allowance.get(key) ?? 0) + entry.count)
    const actual = seenOccurrences.get(key)?.size ?? 0
    if (actual < entry.count) stale.push(`${entry.file}: ${JSON.stringify(entry.text)} (${entry.owner}): pardoned ${entry.count}, found ${actual}`)
  }
  const open: Violation[] = []
  for (const v of violations) {
    const key = `${v.file}\u0000${v.text}`
    const room = allowance.get(key) ?? 0
    const at = occurrence(v)
    if (room === 0) {
      open.push(v)
      continue
    }
    // The first `room` distinct occurrences of the string are pardoned; any rule hit on them is pardoned with them.
    const order = [...(seenOccurrences.get(key) ?? [])].sort().indexOf(at)
    if (order >= room) open.push(v)
  }
  return { open, stale }
}

export function describeViolation(v: Violation): string {
  return `${v.file}:${v.line} [${v.rule}] (${v.how}) ${JSON.stringify(v.text)}`
}

// ── Rust: the sentences a command answers with ──────────────────────────────

/**
 * Rust words reach the screen too: a command's `Err("…")` is the `reason` of a toast. The scan reads production Rust (test
 * items and comments removed; Windows and Linux file systems left out) for SENTENCES: a string literal of three or more
 * words that starts with a capital and ends with a full stop, a question mark or an exclamation mark, outside a log line,
 * an assertion or a panic. That is how this crate writes what a person may read; `format!("open state db: {e}")` is a
 * technical prefix and is a separate matter.
 *
 * "engine" is not checked here: §10.1 exempts Rust's own refusal sentences ("The sync engine did not confirm it stopped.
 * Restart Beebeeb."), which are already person-facing and which specs 1887, 1882 and C keep word for word.
 */
export interface RustSentence {
  file: string
  line: number
  text: string
}

const RUST_EXEMPT_RULES = new Set(['engine'])

function rustFiles(repo: string): string[] {
  const root = join(repo, 'src-tauri/src')
  const out: string[] = []
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name)
      if (statSync(path).isDirectory()) {
        if (name !== 'windows_cf' && name !== 'linux_fuse') walk(path)
      } else if (path.endsWith('.rs')) {
        out.push(path)
      }
    }
  }
  if (existsSync(root)) walk(root)
  return out.sort()
}

/** The sentences of one Rust source (test items and comments already out of the way). */
export function rustSentencesIn(file: string, source: string): RustSentence[] {
  const found: RustSentence[] = []
  stripComments(stripRustTestItems(source)).split('\n').forEach((line, index) => {
    if (/tracing::|eprintln!|println!|assert|panic!|\.expect\(|unreachable!/.test(line)) return
    for (const match of line.matchAll(/"((?:[^"\\]|\\.)*)"/g)) {
      const text = match[1]
      if (/^[A-Z][^"]*[.!?]$/.test(text) && text.split(/\s+/).length >= 3) found.push({ file, line: index + 1, text })
    }
  })
  return found
}

export function scanRust(repo: string): { files: number; sentences: number; violations: Violation[] } {
  const violations: Violation[] = []
  let sentences = 0
  const files = rustFiles(repo)
  for (const path of files) {
    for (const entry of rustSentencesIn(relative(repo, path), readFileSync(path, 'utf8'))) {
      sentences += 1
      for (const word of DEVELOPER_WORDS) {
        if (!RUST_EXEMPT_RULES.has(word.id) && word.pattern.test(entry.text)) violations.push({ ...entry, how: 'rust sentence', rule: word.id })
      }
    }
  }
  return { files: files.length, sentences, violations }
}
