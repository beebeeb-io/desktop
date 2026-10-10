/**
 * A static census of every place in `src/` that invokes a Finder-related Tauri command, and of what
 * each one does with the result on a Mac (task 17b, fix round 1).
 *
 * Why it exists: on a Mac every error from the File Provider bridge that reaches the frontend is a
 * bare domain and code, and a successful repair can still carry a `warnings` list holding such a code
 * and a cache path. Neither may be shown to a person. The first sweep listed FILES, which proved that a
 * file named a command and nothing about what it rendered, and it missed two paths. This one reads
 * CALL SITES: for each invocation it finds the variable that holds the result, finds every use of it,
 * decides whether that use can run on a Mac, and reports the ones that render the reason or the
 * warnings (a property read, an element read, a destructuring, an alias, or a hand-off to a function
 * that is not a known sanitiser).
 *
 * What it cannot see, and says so: a result stored in a field or a ref and read from another function;
 * a command named by a computed string (a separate check counts every string literal that names one);
 * a guard it does not recognise (it then assumes the use IS reachable on a Mac, so a missed guard fails
 * loudly instead of passing quietly). A `.then` chain on the call is followed one level.
 *
 * It is pure: it takes source text, so the tests can prove it on snippets that leak, and on the real
 * tree, before anyone trusts it to pass.
 */
import ts from 'typescript'

/** A command whose name contains one of these is Finder-related. `finder_setup_` is a prefix. */
export const COMMAND_FAMILIES = [
  'open_finder_location',
  'reset_macos_integration',
  'finder_setup_',
  'open_in_finder',
  'open_login_items_and_extensions_settings',
] as const

/** Functions that hand a Finder command's result back to a caller: the caller is a call site too. */
export const WRAPPERS = ['loadFinderSetup', 'copyFinderSetupDetails', 'runFinderSetupAction'] as const

/**
 * Functions that may be handed a result (or its `.value`) because they turn it into fixed words and
 * never return what they were given. Each is pinned by its own unit test.
 */
export const SANITISERS = ['repairNote', 'finderRepairWarningNote', 'parseFinderSetupView'] as const

/**
 * Task 1882 (rebase onto main, lead ruling): functions that may be handed a repair's result `.value` because they read
 * only its `preserved_location`, the folder where macOS kept files that had not reached the server, and return that
 * path, never the reason or the warnings. They are not sanitisers (the folder IS shown, on purpose), so they have their
 * own list. `tests/finderSetupSourceContract.test.ts` (exemption 4) pins that each reads nothing else.
 */
export const KEPT_FOLDER_READERS = ['preservedFilesNote', 'preservedFilesLine'] as const

/** The families whose SUCCESS payload carries free text (a repair's `warnings`): `.value` is strict. */
const STRICT_VALUE_FAMILIES = new Set(['reset_macos_integration'])

export type ViolationKind = 'reason' | 'warnings' | 'computed' | 'destructure' | 'alias' | 'passed-on' | 'unanalysed'

export interface Violation {
  kind: ViolationKind
  /** The source text of the offending use, trimmed to a line. */
  text: string
  /** False when the use sits only on a non-macOS side of a recognised guard. */
  macReachable: boolean
}

export interface Site {
  file: string
  /** One of COMMAND_FAMILIES, `wrapper:<name>`, or `finder.run`. */
  family: string
  line: number
  /** The text of the call, trimmed to a line. */
  call: string
  /** `bound` to a name (or a `.then` parameter), `returned`, or `discarded`. Anything else is `unanalysed`. */
  consumption: 'bound' | 'returned' | 'discarded' | 'unanalysed'
  /** Every leak found, split by `macReachable`. */
  violations: Violation[]
}

type Side = 'mac' | 'notmac' | null

function unwrap(node: ts.Node): ts.Node {
  let current = node
  while (
    ts.isParenthesizedExpression(current) ||
    ts.isNonNullExpression(current) ||
    ts.isAsExpression(current) ||
    ts.isAwaitExpression(current) ||
    ts.isVoidExpression(current)
  ) {
    current = current.expression
  }
  return current
}

const isMacLiteral = (node: ts.Node) => ts.isStringLiteralLike(node) && node.text === 'macos'
const MAC_IDENTIFIER = /^(isMac|isMacos|isMacOS|mac)$/

/** What a condition says about the branch it guards: `then` is its true side, `otherwise` its false side. */
function guardSides(expr: ts.Node): { then: Side; otherwise: Side } {
  const e = unwrap(expr)
  if (ts.isIdentifier(e) && MAC_IDENTIFIER.test(e.text)) return { then: 'mac', otherwise: 'notmac' }
  if (ts.isPrefixUnaryExpression(e) && e.operator === ts.SyntaxKind.ExclamationToken) {
    const inner = guardSides(e.operand)
    return { then: inner.otherwise, otherwise: inner.then }
  }
  if (ts.isBinaryExpression(e)) {
    const op = e.operatorToken.kind
    const compares = isMacLiteral(unwrap(e.left)) || isMacLiteral(unwrap(e.right))
    if (compares && (op === ts.SyntaxKind.EqualsEqualsEqualsToken || op === ts.SyntaxKind.EqualsEqualsToken)) return { then: 'mac', otherwise: 'notmac' }
    if (compares && (op === ts.SyntaxKind.ExclamationEqualsEqualsToken || op === ts.SyntaxKind.ExclamationEqualsToken)) return { then: 'notmac', otherwise: 'mac' }
    const left = guardSides(e.left)
    const right = guardSides(e.right)
    const strongest = (a: Side, b: Side): Side => (a === 'notmac' || b === 'notmac' ? 'notmac' : a === 'mac' || b === 'mac' ? 'mac' : null)
    // `a && b` is true only if both are: its true side is known, its false side is not.
    if (op === ts.SyntaxKind.AmpersandAmpersandToken) return { then: strongest(left.then, right.then), otherwise: null }
    // `a || b` is false only if both are.
    if (op === ts.SyntaxKind.BarBarToken) return { then: null, otherwise: strongest(left.otherwise, right.otherwise) }
  }
  return { then: null, otherwise: null }
}

/** True unless `node` sits only on the non-macOS side of a guard between it and `scope`. */
function macReachable(node: ts.Node, scope: ts.Node): boolean {
  let child: ts.Node = node
  for (let parent = child.parent; parent && child !== scope; child = parent, parent = parent.parent) {
    let side: Side = null
    if (ts.isConditionalExpression(parent)) {
      const sides = guardSides(parent.condition)
      if (child === parent.whenTrue) side = sides.then
      else if (child === parent.whenFalse) side = sides.otherwise
    } else if (ts.isIfStatement(parent)) {
      const sides = guardSides(parent.expression)
      if (child === parent.thenStatement) side = sides.then
      else if (child === parent.elseStatement) side = sides.otherwise
    } else if (ts.isBinaryExpression(parent) && child === parent.right) {
      const sides = guardSides(parent.left)
      if (parent.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken) side = sides.then
      else if (parent.operatorToken.kind === ts.SyntaxKind.BarBarToken) side = sides.otherwise
    }
    if (side === 'notmac') return false
    if (parent === scope) break
  }
  return true
}

const oneLine = (node: ts.Node, sf: ts.SourceFile) => node.getText(sf).replace(/\s+/g, ' ').slice(0, 140)

function calleeName(call: ts.CallExpression): string | null {
  const callee = call.expression
  if (ts.isIdentifier(callee)) return callee.text
  if (ts.isPropertyAccessExpression(callee)) return callee.name.text
  return null
}

function enclosingFunction(node: ts.Node): ts.Node {
  let current: ts.Node = node
  while (current.parent && !ts.isFunctionLike(current)) current = current.parent
  return current
}

function bindingNames(pattern: ts.BindingName): { names: string[]; rest: boolean } {
  const names: string[] = []
  let rest = false
  const visit = (name: ts.BindingName) => {
    if (!ts.isObjectBindingPattern(name) && !ts.isArrayBindingPattern(name)) return
    for (const element of name.elements) {
      if (ts.isOmittedExpression(element)) continue
      if (element.dotDotDotToken) rest = true
      const key = element.propertyName ?? element.name
      if (ts.isIdentifier(key)) names.push(key.text)
      else if (ts.isStringLiteralLike(key)) names.push(key.text)
      visit(element.name)
    }
  }
  visit(pattern)
  return { names, rest }
}

/** Every use of the identifier `name` inside `scope` that reads the variable rather than declaring it. */
function references(scope: ts.Node, name: string, declaration: ts.Node | null): ts.Identifier[] {
  const found: ts.Identifier[] = []
  const visit = (node: ts.Node) => {
    if (ts.isIdentifier(node) && node.text === name && node !== declaration) {
      const parent = node.parent
      const isMemberName = ts.isPropertyAccessExpression(parent) && parent.name === node
      const isKey = (ts.isPropertyAssignment(parent) || ts.isPropertySignature(parent)) && parent.name === node
      const isBindingName = ts.isBindingElement(parent) && (parent.name === node || parent.propertyName === node)
      const isParam = ts.isParameter(parent) && parent.name === node
      if (!isMemberName && !isKey && !isBindingName && !isParam) found.push(node)
    }
    ts.forEachChild(node, visit)
  }
  visit(scope)
  return found
}

function classify(id: ts.Identifier, family: string): { kind: ViolationKind; node: ts.Node } | null {
  let outer: ts.Node = id
  const path: string[] = []
  for (;;) {
    const parent = outer.parent
    if (ts.isPropertyAccessExpression(parent) && parent.expression === outer) {
      path.push(parent.name.text)
      outer = parent
    } else if (ts.isElementAccessExpression(parent) && parent.expression === outer) {
      path.push(ts.isStringLiteralLike(parent.argumentExpression) ? parent.argumentExpression.text : '[computed]')
      outer = parent
    } else if (ts.isNonNullExpression(parent) || ts.isParenthesizedExpression(parent) || ts.isAsExpression(parent)) {
      outer = parent
    } else break
  }
  if (path.includes('reason')) return { kind: 'reason', node: outer }
  if (path.includes('warnings')) return { kind: 'warnings', node: outer }
  if (path.includes('[computed]')) return { kind: 'computed', node: outer }

  const parent = outer.parent
  // `const { reason } = result`, `const { warnings } = result.value`, or any rest element.
  if (ts.isVariableDeclaration(parent) && parent.initializer === outer && !ts.isIdentifier(parent.name)) {
    const { names, rest } = bindingNames(parent.name)
    return rest || names.includes('reason') || names.includes('warnings') ? { kind: 'destructure', node: parent } : null
  }
  const bareResult = path.length === 0
  const bareValue = path.length === 1 && path[0] === 'value' && STRICT_VALUE_FAMILIES.has(family)
  if (!bareResult && !bareValue) return null
  if (ts.isReturnStatement(parent)) return null
  if (ts.isArrowFunction(parent) && parent.body === outer) return null
  if (ts.isCallExpression(parent) && parent.arguments.includes(outer as ts.Expression)) {
    const callee = calleeName(parent)
    const allowed = [...SANITISERS, ...KEPT_FOLDER_READERS] as readonly string[]
    return callee && allowed.includes(callee) ? null : { kind: 'passed-on', node: parent }
  }
  // An alias, a spread, a field, an argument position we do not know: the reason travels with it.
  return { kind: ts.isVariableDeclaration(parent) ? 'alias' : 'passed-on', node: parent }
}

function analyse(call: ts.CallExpression, family: string, file: string, sf: ts.SourceFile): Site {
  const site: Site = {
    file,
    family,
    line: sf.getLineAndCharacterOfPosition(call.getStart(sf)).line + 1,
    call: oneLine(call, sf),
    consumption: 'unanalysed',
    violations: [],
  }
  // Climb through `await`, parentheses and `as`, to see who receives the value.
  let outer: ts.Node = call
  while (ts.isAwaitExpression(outer.parent) || ts.isParenthesizedExpression(outer.parent) || ts.isAsExpression(outer.parent) || ts.isNonNullExpression(outer.parent)) {
    outer = outer.parent
  }
  const parent = outer.parent
  let scope: ts.Node | null = null
  let name: string | null = null
  let declaration: ts.Node | null = null

  if (ts.isVariableDeclaration(parent) && parent.initializer === outer) {
    if (ts.isIdentifier(parent.name)) {
      name = parent.name.text
      declaration = parent.name
      scope = enclosingFunction(parent)
      site.consumption = 'bound'
    } else {
      // `const { ok, reason } = await command(...)`: report the pattern itself.
      const { names, rest } = bindingNames(parent.name)
      site.consumption = 'bound'
      if (rest || names.includes('reason') || names.includes('warnings')) {
        site.violations.push({ kind: 'destructure', text: oneLine(parent, sf), macReachable: macReachable(parent, enclosingFunction(parent)) })
      }
    }
  } else if (ts.isPropertyAccessExpression(parent) && parent.expression === outer && parent.name.text === 'then' && ts.isCallExpression(parent.parent)) {
    const callback = parent.parent.arguments[0]
    if (callback && (ts.isArrowFunction(callback) || ts.isFunctionExpression(callback)) && callback.parameters.length > 0) {
      const param = callback.parameters[0]
      if (ts.isIdentifier(param.name)) {
        name = param.name.text
        declaration = param.name
        scope = callback
        site.consumption = 'bound'
      }
    }
  } else if (ts.isReturnStatement(parent) || (ts.isArrowFunction(parent) && parent.body === outer)) {
    site.consumption = 'returned'
  } else if (ts.isExpressionStatement(parent)) {
    site.consumption = 'discarded'
  } else if (ts.isVoidExpression(parent)) {
    site.consumption = 'discarded'
  }

  if (name && scope) {
    for (const id of references(scope, name, declaration)) {
      const found = classify(id, family)
      if (found) site.violations.push({ kind: found.kind, text: oneLine(found.node, sf), macReachable: macReachable(id, scope) })
    }
  }
  if (site.consumption === 'unanalysed') {
    site.violations.push({ kind: 'unanalysed', text: oneLine(parent, sf), macReachable: true })
  }
  return site
}

const stringLiteralFamily = (text: string): string | null => COMMAND_FAMILIES.find((family) => text.includes(family)) ?? null

/** Every call site in one source file. `file` is the path relative to `src/`. */
export function censusOfSource(file: string, text: string): Site[] {
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const sites: Site[] = []
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node)) {
      const callee = node.expression
      const first = node.arguments[0]
      if (ts.isIdentifier(callee) && callee.text === 'command' && first && ts.isStringLiteralLike(first)) {
        const family = stringLiteralFamily(first.text)
        if (family) sites.push(analyse(node, family === 'finder_setup_' ? 'finder_setup_*' : family, file, sf))
      } else if (ts.isIdentifier(callee) && (WRAPPERS as readonly string[]).includes(callee.text)) {
        sites.push(analyse(node, `wrapper:${callee.text}`, file, sf))
      } else if (ts.isPropertyAccessExpression(callee) && callee.name.text === 'run' && ts.isIdentifier(callee.expression) && callee.expression.text === 'finder') {
        sites.push(analyse(node, 'finder.run', file, sf))
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  return sites
}

/**
 * Every string literal in the file that names a family, calls or not. A command reached through a
 * variable (`const name = 'open_in_finder'; command(name)`) shows here as a literal with no call site.
 */
export function familyLiterals(file: string, text: string): Record<string, number> {
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const counts: Record<string, number> = {}
  const visit = (node: ts.Node) => {
    if (ts.isStringLiteralLike(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) {
      const family = stringLiteralFamily(node.text)
      if (family) counts[family === 'finder_setup_' ? 'finder_setup_*' : family] = (counts[family === 'finder_setup_' ? 'finder_setup_*' : family] ?? 0) + 1
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  return counts
}
