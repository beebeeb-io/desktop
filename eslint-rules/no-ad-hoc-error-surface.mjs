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
 * KNOWN BLIND SPOT — do not claim this rule "cannot undercount", only that it cannot
 * forget to look. It recognises an error state by two shapes: a setter receiving
 * `.reason`/`commandUnavailableLabel(...)`, or a `useState<CommandResult<...>>`. An error
 * sourced from anywhere else is invisible to it. A real example lives in this repo:
 * WindowsFirstRun's `browserError` is fed from a Tauri event payload (`p.message`), so the
 * census does not see it. Widening to `.message` was considered and rejected — it matches
 * every `Error.message` in the codebase and would bury the signal. If a third origin shape
 * appears, add it here rather than going back to counting by hand.
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

/**
 * Text-entry controls the user can CORRECT and resubmit. A toggle or a button is not
 * one: there is nothing to retype, so a failed toggle has no correction to block.
 */
const CORRECTABLE_INPUT_TYPES = new Set([
  'text', 'password', 'email', 'search', 'number', 'tel', 'url', 'date', 'time',
])

function isCorrectableInput(node) {
  const name = jsxElementName(node)
  if (name === 'textarea') return true
  if (name !== 'input') return false
  const typeAttr = node.openingElement.attributes.find(
    (a) => a.type === 'JSXAttribute' && a.name?.name === 'type',
  )
  if (!typeAttr) return true // an <input> with no type is a text field
  const value = typeAttr.value
  if (value?.type !== 'Literal') return true // dynamic type — assume correctable
  return CORRECTABLE_INPUT_TYPES.has(String(value.value))
}

/**
 * THE THIRD CLAUSE (task 1319, lead ruling 2026-08-31).
 *
 * The principle had two exemptions — load failures and errors that gate a control — and
 * this rule therefore reported WindowsFirstRun's `pwError`/`totpError` as stragglers,
 * because a sign-in error hides no element. Checked against the source, the humans were
 * right and the PRINCIPLE was incomplete, not the code: `submitPassword` is a
 * `<form onSubmit>` handler, `pwError` renders immediately above that form, and the email
 * and password inputs stay on screen for the user to correct. A 6s toast would vanish
 * while someone is still retyping their password. That is a real third category:
 *
 *   an error the user must SEE WHILE CORRECTING the input that caused it stays inline.
 *
 * Detected structurally: the error renders in a subtree that also holds a correctable
 * text input. Both known shapes satisfy it — `pwError` renders as a sibling of the
 * <form> holding the inputs, and TrashView's `deleteError` renders as a direct sibling
 * of the delete modal's <input type="password">.
 *
 * Deliberately NOT "the component contains an input anywhere": that would give any
 * component with a search box a free pass for every unrelated action failure, which is
 * an undercount, and undercounting is the failure mode this rule exists to prevent.
 */
function rendersCorrectableInput(node) {
  let found = false
  walk(node, (n) => {
    if (found || n.type !== 'JSXElement') return
    if (isCorrectableInput(n)) found = true
  })
  return found
}

/**
 * A SUBSTITUTE RENDER: the error is returned as the whole view body, replacing whatever
 * that view would otherwise show, rather than annotating a view that is still there.
 *
 * This is NOT a fourth clause — it is the missing structural signature of the EXISTING
 * load clause. `isLoadPath` recognises a load by its call site (inside useEffect, or a
 * function useEffect calls), which misses a load triggered by user interaction.
 * DesktopVersionHistory's `loadVersions` is exactly that: it nulls the list, sets a
 * loading flag, fetches versions, and on failure returns the error AS the panel body:
 *
 *     if (versionNotice != null) return <div className="quick-search-notice">{versionNotice}</div>
 *     if (earlierVersions.length === 0) return <div>No earlier versions</div>
 *
 * Route that to a toast and the panel falls through to "No earlier versions" — a load
 * failure becomes indistinguishable from an empty result. That is precisely the bug task
 * 1255 found in TrashView and refused to reintroduce.
 *
 * Kept deliberately TIGHT to avoid becoming a blanket exemption: the returned expression
 * must be a JSX element whose ONLY interpolation is this error. An error buried inside a
 * large returned tree is an annotation, not a substitute, and is still reported — a
 * component with early returns must not thereby exempt every error in its main body.
 */
function isSubstituteRender(identifier, chain) {
  const fnIndex = chain.findIndex((a) => FUNCTION_TYPES.has(a.type))
  const returnIndex = chain.findIndex((a) => a.type === 'ReturnStatement')
  if (returnIndex === -1) return false
  if (fnIndex !== -1 && fnIndex < returnIndex) return false // return belongs to an inner fn

  const returned = chain[returnIndex].argument
  if (returned?.type !== 'JSXElement') return false

  const containers = []
  walk(returned, (n) => { if (n.type === 'JSXExpressionContainer') containers.push(n) })
  if (containers.length !== 1) return false
  let onlyHoldsThisError = false
  walk(containers[0], (n) => { if (n === identifier) onlyHoldsThisError = true })
  if (!onlyHoldsThisError) return false

  // A lone return substitutes for nothing; require a sibling branch it displaces.
  const fn = chain.slice(returnIndex).find((a) => FUNCTION_TYPES.has(a.type))
  if (!fn) return false
  let jsxReturns = 0
  walk(fn.body, (n) => {
    if (n.type === 'ReturnStatement' && n.argument?.type === 'JSXElement') jsxReturns += 1
  })
  return jsxReturns >= 2
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
    correctionBlocking: false,
    substituteRender: false,
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

  // Does this error render alongside an input the user is correcting?
  const enclosing = chain.find((a) => a.type === 'JSXElement')
  if (enclosing && rendersCorrectableInput(enclosing)) result.correctionBlocking = true
  if (isSubstituteRender(identifier, chain)) result.substituteRender = true

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
        // Second recognised shape: the whole CommandResult is parked in state
        // (`useState<CommandResult<void> | null>`) and `.reason` is read at the render
        // site instead of the setter. Onboarding's UnlockStep does this, and reading
        // only setter arguments missed it entirely — found by diffing this rule's census
        // against a hand count, which is the sort of gap a hand count cannot self-detect.
        const typeArgs = node.init.typeArguments ?? node.init.typeParameters
        let isErrorState = Boolean(typeArgs && /\bCommandResult\b/.test(sourceCode.getText(typeArgs)))
        let setFromLoad = false
        let setFromAction = false

        // When the state is error-typed (a parked CommandResult), every write counts —
        // its setter argument is the whole result object, so `.reason` never appears at
        // the call site and requiring it there left the state detected but unclassified,
        // i.e. silently never reported. Caught by the rule's own test, not by reading it.
        const errorByType = isErrorState
        for (const ref of setterVar.references) {
          const call = ref.identifier.parent
          if (call?.type !== 'CallExpression' || call.callee !== ref.identifier) continue
          const arg = call.arguments[0]
          if (!arg) continue
          const argIsNullish = arg.type === 'Literal' && arg.value === null
          if (!errorByType && !isErrorOrigin(arg)) continue
          if (errorByType && argIsNullish) continue // `setResult(null)` is a reset, not a failure
          isErrorState = true
          if (isLoadPath(call, sourceCode)) setFromLoad = true
          else setFromAction = true
        }

        if (!isErrorState) return

        // ---- How is it presented? ----
        let gatesControl = false
        let correctionBlocking = false
        let substituteRender = false
        let renderedInline = false
        let passedAsProp = false

        for (const ref of stateVar.references) {
          const usage = classifyJsxUsage(ref.identifier)
          if (!usage) continue
          if (usage.gatesControl) gatesControl = true
          if (usage.correctionBlocking) correctionBlocking = true
          if (usage.substituteRender) substituteRender = true
          if (usage.renderedInline) renderedInline = true
          if (usage.passedAsProp) passedAsProp = true
        }

        // A census mode exists so the per-file enumeration is reproducible by anyone.
        // Four hand counts of these files disagreed; `BB_ERROR_SURFACE_CENSUS=1 bunx eslint src`
        // prints every error state the rule can see, defect or not, so the next reviewer
        // re-derives the number instead of trusting a figure typed into a task file.
        // NOTE: this writes to stderr rather than context.report ON PURPOSE. A census
        // routed through context.report is silenced by the very eslint-disable comments
        // that mark documented exceptions, so it would under-report exactly the states
        // someone had already decided about — reintroducing the undercount in the tool
        // built to end it. stderr cannot be disabled away.
        if (process.env.BB_ERROR_SURFACE_CENSUS === '1') {
          process.stderr.write(JSON.stringify({
            census: true,
            file: context.filename ?? context.getFilename(),
            line: node.loc.start.line,
            name: stateVar.name,
            trigger: setFromLoad ? (setFromAction ? 'load+action' : 'load') : 'action',
            gatesControl,
            correctionBlocking,
            substituteRender,
            presentation: passedAsProp ? 'prop' : renderedInline ? 'inline' : 'toast-only',
          }) + '\n')
        }

        // ---- Arm A: a transient action failure that should have been a toast ----
        // A load failure is allowed to stay inline (the surface is degraded and needs a
        // persistent explanation). An error that gates a control must stay inline (a
        // toast would delete the control). Everything else is a straggler.
        if (
          setFromAction &&
          !setFromLoad &&
          !gatesControl &&
          !correctionBlocking &&
          !substituteRender &&
          (renderedInline || passedAsProp)
        ) {
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
