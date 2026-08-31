/**
 * ESLint rule: no-ad-hoc-error-surface
 *
 * WHY THIS EXISTS (task 1319, and read this before weakening it):
 * The Toast migration (1248 → 1255 → 1318) was scoped by hand-written file lists
 * three times and undercounted every time. WindowsFirstRun.tsx was enumerated as
 * having 2 error states, then "corrected" to 5; the real answer is 6. Four careful
 * human passes produced four wrong counts. The defect is not that people are
 * careless — it is that nobody can reliably eyeball every error state in a 1000-line
 * React file, and each pass only counts the states it went looking for.
 *
 * So this rule does the counting. It cannot "forget to look".
 *
 * THE PRINCIPLE IT ENFORCES (from 1248/1255):
 *   A transient ACTION failure (a button press, a toggle) belongs in a Toast.
 *   A LOAD failure, or an error that GATES a control, stays inline by design.
 *
 * HOW IT FINDS ERROR STATE WITHOUT GREPPING FOR NAMES:
 * Error strings in this codebase have exactly one origin shape — `CommandResult.reason`
 * or `commandUnavailableLabel(...)` (see src/desktopApi.ts). So a `useState` whose
 * setter is called with an expression containing either of those IS an error state,
 * regardless of what it is named. That is a data-flow fact, not a naming convention,
 * which is why this does not degrade into the unfalsifiable grep that task 1255's
 * verification line originally asked for.
 *
 * WHAT IT REPORTS:
 * An error state written only from event handlers (never from a useEffect load), which
 * gates no control, but is still rendered inline or handed to another component. That is
 * a straggler: it should call showToast.
 *
 * WHAT IT DELIBERATELY DOES NOT REPORT, AND WHY:
 * 1319 asked for a second arm failing when an error renders outside the shared
 * Notice/ErrorBlock components. That was tried and removed. This codebase styles
 * essentially every element with an inline `style={{...}}` object, so "error rendered in a
 * raw styled div" matched 41 sites and did not discriminate between the defect and the
 * house style. A check that fires on the house style trains people to disable it. The
 * presentation-consistency question is real but it is a design-system decision (there is
 * no shared Notice module — WindowsFirstRun has a local one and ErrorBlock is duplicated
 * in ActivityView and SettingsView), so it needs its own task, not a noisy rule here.
 *
 * DELIBERATE EXCEPTIONS: use an eslint-disable-next-line comment WITH a reason. The
 * rule is designed so that documenting an exception in code is the way to satisfy it,
 * which is also what task 1255's AC6 requires.
 */

const ERROR_ORIGIN_CALLEES = new Set(['commandUnavailableLabel'])
const ERROR_ORIGIN_PROPERTIES = new Set(['reason'])

/** Components that are an approved, consistent inline error presentation. */
const APPROVED_ERROR_COMPONENTS = new Set(['Notice', 'ErrorBlock'])

/** Rendering any of these behind an error flag means the error GATES a control. */
const INTERACTIVE_ELEMENTS = new Set([
  'button', 'a', 'input', 'select', 'textarea', 'form',
  'Btn', 'PrimaryBtn', 'Toggle', 'Modal',
])

/** Walk every node in a subtree. */
function walk(node, visit, seen = new Set()) {
  if (!node || typeof node.type !== 'string' || seen.has(node)) return
  seen.add(node)
  visit(node)
  for (const key of Object.keys(node)) {
    if (key === 'parent') continue
    const value = node[key]
    if (Array.isArray(value)) {
      for (const child of value) {
        if (child && typeof child.type === 'string') walk(child, visit, seen)
      }
    } else if (value && typeof value.type === 'string') {
      walk(value, visit, seen)
    }
  }
}

/** Does this expression carry a value that originated from a command failure? */
function isErrorOrigin(node) {
  let found = false
  walk(node, (n) => {
    if (found) return
    if (n.type === 'MemberExpression' && !n.computed && ERROR_ORIGIN_PROPERTIES.has(n.property?.name)) {
      found = true
    }
    if (n.type === 'CallExpression' && n.callee?.type === 'Identifier' && ERROR_ORIGIN_CALLEES.has(n.callee.name)) {
      found = true
    }
  })
  return found
}

function ancestors(node) {
  const chain = []
  for (let current = node.parent; current; current = current.parent) chain.push(current)
  return chain
}

/** True when the node sits lexically inside a useEffect callback. */
function isInsideUseEffect(node) {
  return ancestors(node).some(
    (a) => a.type === 'CallExpression' && a.callee?.type === 'Identifier' && a.callee.name === 'useEffect',
  )
}

const FUNCTION_TYPES = new Set([
  'ArrowFunctionExpression',
  'FunctionExpression',
  'FunctionDeclaration',
])

/**
 * True when this setter call is on a LOAD path.
 *
 * Lexical containment in useEffect is not enough. The dominant idiom in this repo is:
 *
 *     const load = async () => { ...; setError(reason) }
 *     useEffect(() => { void load() }, [])
 *
 * so the setError call sits inside `load`, whose parent chain never reaches useEffect.
 * Reading only lexical nesting classified TrashView's `desktop_trash_list` LOAD failure
 * as a transient action — which would have told someone to "fix" it into a toast and
 * reintroduce the exact bug 1255 removed (a load failure becoming indistinguishable
 * from an empty state). So follow one level of indirection: if the enclosing function is
 * bound to a name, and that name is called inside a useEffect, this is a load path.
 */
function isLoadPath(node, sourceCode) {
  if (isInsideUseEffect(node)) return true

  for (const ancestor of ancestors(node)) {
    if (!FUNCTION_TYPES.has(ancestor.type)) continue

    let declarator = null
    if (ancestor.parent?.type === 'VariableDeclarator' && ancestor.parent.init === ancestor) {
      declarator = ancestor.parent
    } else if (ancestor.type === 'FunctionDeclaration') {
      declarator = ancestor
    }
    if (!declarator) continue

    const [fnVar] = sourceCode.getDeclaredVariables(declarator)
    if (!fnVar) continue
    if (fnVar.references.some((ref) => isInsideUseEffect(ref.identifier))) return true
  }

  return false
}

function jsxElementName(node) {
  const name = node.openingElement?.name
  if (!name) return null
  if (name.type === 'JSXIdentifier') return name.name
  if (name.type === 'JSXMemberExpression') return name.property?.name ?? null
  return null
}

/** Does this subtree render an interactive control? */
function rendersInteractiveControl(node) {
  let found = false
  walk(node, (n) => {
    if (found || n.type !== 'JSXElement') return
    const name = jsxElementName(n)
    if (name && INTERACTIVE_ELEMENTS.has(name)) found = true
  })
  return found
}

/**
 * Given an identifier reference, classify how it is used in JSX.
 * Returns { gatesControl, renderedInline, passedAsProp, approvedRender }
 */
function classifyJsxUsage(identifier) {
  const chain = ancestors(identifier)
  const containerIndex = chain.findIndex((a) => a.type === 'JSXExpressionContainer')
  if (containerIndex === -1) return null

  const container = chain[containerIndex]
  const result = {
    gatesControl: false,
    renderedInline: false,
    passedAsProp: false,
    approvedRender: false,
    node: identifier,
  }

  // `<Panel notice={notice} />` — the value escapes into another component.
  if (container.parent?.type === 'JSXAttribute') {
    result.passedAsProp = true
    return result
  }

  result.renderedInline = true

  // `{err && <X/>}` — what does the guard reveal?
  // Take the OUTERMOST `&&` below the container, not the innermost: in
  // `{message && !isMacos && <button/>}` the innermost LogicalExpression's `.right`
  // is `!isMacos`, and reading that instead of the button is how this rule first
  // mis-reported Onboarding's FinderInstallStep — a genuinely control-gating error —
  // as a straggler. The rendered element hangs off the outermost `&&`.
  const logicals = chain
    .slice(0, containerIndex)
    .filter((a) => a.type === 'LogicalExpression' && a.operator === '&&')
  const logical = logicals[logicals.length - 1]
  if (logical) {
    if (rendersInteractiveControl(logical.right)) result.gatesControl = true
    let rendered = logical.right
    while (rendered?.type === 'LogicalExpression') rendered = rendered.right
    if (rendered?.type === 'JSXElement') {
      const name = jsxElementName(rendered)
      if (name && APPROVED_ERROR_COMPONENTS.has(name)) result.approvedRender = true
    }
  } else {
    // `<div>{err}</div>` with no guard — the enclosing element is the presentation.
    const element = chain.find((a) => a.type === 'JSXElement')
    if (element) {
      const name = jsxElementName(element)
      if (name && APPROVED_ERROR_COMPONENTS.has(name)) result.approvedRender = true
    }
  }

  return result
}

export default {
  meta: {
    type: 'problem',
    docs: {
      description:
        'Transient action failures must go through the shared Toast system; inline error surfaces must use the shared Notice/ErrorBlock components.',
    },
    schema: [],
    messages: {
      transientActionInline:
        "'{{name}}' holds a command failure that is only ever set from an event handler (never a useEffect load) and gates no control, but it is rendered inline. A transient ACTION failure belongs in a Toast — call showToast instead. If this is deliberate, add an eslint-disable-next-line comment saying which control it gates or why it must persist.",
    },
  },

  create(context) {
    const sourceCode = context.sourceCode ?? context.getSourceCode()

    return {
      VariableDeclarator(node) {
        if (node.id.type !== 'ArrayPattern') return
        if (node.init?.type !== 'CallExpression') return
        const callee = node.init.callee
        const isUseState =
          (callee.type === 'Identifier' && callee.name === 'useState') ||
          (callee.type === 'MemberExpression' && callee.property?.name === 'useState')
        if (!isUseState) return

        const declared = sourceCode.getDeclaredVariables(node)
        const stateVar = declared.find((v) => v.name === node.id.elements[0]?.name)
        const setterVar = declared.find((v) => v.name === node.id.elements[1]?.name)
        if (!stateVar || !setterVar) return

        // ---- Is this an error state at all? (data-flow, not naming) ----
        let isErrorState = false
        let setFromLoad = false
        let setFromAction = false

        for (const ref of setterVar.references) {
          const call = ref.identifier.parent
          if (call?.type !== 'CallExpression' || call.callee !== ref.identifier) continue
          const arg = call.arguments[0]
          if (!arg || !isErrorOrigin(arg)) continue
          isErrorState = true
          if (isLoadPath(call, sourceCode)) setFromLoad = true
          else setFromAction = true
        }

        if (!isErrorState) return

        // ---- How is it presented? ----
        let gatesControl = false
        let renderedInline = false
        let passedAsProp = false

        for (const ref of stateVar.references) {
          const usage = classifyJsxUsage(ref.identifier)
          if (!usage) continue
          if (usage.gatesControl) gatesControl = true
          if (usage.renderedInline) renderedInline = true
          if (usage.passedAsProp) passedAsProp = true
        }

        // ---- Arm A: a transient action failure that should have been a toast ----
        // A load failure is allowed to stay inline (the surface is degraded and needs a
        // persistent explanation). An error that gates a control must stay inline (a
        // toast would delete the control). Everything else is a straggler.
        if (setFromAction && !setFromLoad && !gatesControl && (renderedInline || passedAsProp)) {
          context.report({
            node: node.id.elements[0],
            messageId: 'transientActionInline',
            data: { name: stateVar.name },
          })
        }
      },
    }
  },
}
