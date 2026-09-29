import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, type ReactElement } from 'react'
import ts from 'typescript'
import { renderToStaticMarkup } from 'react-dom/server'
import OnboardingErrorBoundary from '../src/OnboardingErrorBoundary'

// Exercise the production UnlockStep JSX and handlers without a DOM dependency.
// React clears currentTarget after dispatch; queued updaters must still work.
// This controlled scheduler explicitly covers both eager and deferred updates.
// Browser focus/events, React boundary capture and native IPC remain device gates.
type Element = ReactElement<Record<string, any>>
function component(source: string, name: string, bindings: Record<string, unknown>) {
  const ast = ts.createSourceFile('component.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const declaration = ast.statements.find((node) =>
    (ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node)) && node.name?.text === name,
  )
  if (!declaration) throw new Error(`Missing production component ${name}`)
  const compiled = ts.transpileModule(declaration.getText(ast).replace(/^export default /, '').replace(/^export /, ''), {
    compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText
  return new Function(...Object.keys(bindings), `${compiled}; return ${name}`)(...Object.values(bindings))
}
function elements(node: any): Element[] {
  if (Array.isArray(node)) return node.flatMap(elements)
  if (!node || typeof node !== 'object' || !node.props) return []
  return [node, ...elements(node.props.children)]
}
function text(node: any): string {
  if (Array.isArray(node)) return node.map(text).join(' ')
  if (node && typeof node === 'object') return text(node.props?.children)
  return node == null || typeof node === 'boolean' ? '' : String(node)
}
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
const expectedWords = phrase.split(' ')

function setup(file: string) {
  const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')
  const states: any[] = []
  const updates: Array<() => void> = []
  const frames: Array<() => void> = []
  const refs: any[] = []
  let cursor = 0
  let focus = -1
  let done = 0
  let rejection = ''
  const unlocks: string[] = []
  const Step = component(source, 'UnlockStep', {
    React: { createElement }, RECOVERY_WORD_COUNT: 12, T: {},
    Notice: 'notice', Card: 'card', Btn: 'button',
    useState(initial: any) {
      const index = cursor++
      if (!(index in states)) states[index] = typeof initial === 'function' ? initial() : initial
      return [states[index], (next: any) => updates.push(() => {
        if (typeof next === 'function') {
          // Replay the updater as StrictMode can; neither evaluation retains events.
          next(states[index])
          states[index] = next(states[index])
        } else states[index] = next
      })]
    },
    useRef(initial: any) {
      const index = cursor++
      return refs[index] ??= { current: initial }
    },
    requestAnimationFrame(callback: () => void) { frames.push(callback) },
    commandUnavailableLabel: () => 'Unavailable',
    command: async (name: string, args: { recoveryPhrase: string }) => {
      expect(name).toBe('desktop_unlock_with_recovery_phrase')
      unlocks.push(args.recoveryPhrase)
      return rejection ? { ok: false, reason: rejection, unsupported: false } : { ok: true }
    },
  })
  let tree: Element
  function render() {
    cursor = 0
    tree = Step({ onDone: () => done++ })
    fields().forEach((field, index) => field.props.ref({ focus: () => { focus = index } }))
  }
  function flush() {
    while (updates.length) updates.shift()!()
    render()
    while (frames.length) frames.shift()!()
  }
  function fields() { return elements(tree).filter((el) => el.type === 'input') }
  function change(index: number, value: string, eager = false) {
    const event = { currentTarget: { value } as { value: string } | null }
    fields()[index].props.onChange(event)
    if (eager) flush()
    event.currentTarget = null
    flush()
  }
  function key(index: number, key: string) {
    focus = index
    let prevented = false
    fields()[index].props.onKeyDown({ key, preventDefault() { prevented = true } })
    flush()
    return prevented
  }
  function paste(index: number, value: string) {
    let prevented = false
    fields()[index].props.onPaste({ preventDefault() { prevented = true }, clipboardData: { getData: () => value } })
    flush()
    expect(prevented).toBe(true)
  }
  render()
  return {
    fields, change, key, paste, flush,
    values: () => fields().map((el) => el.props.value),
    focus: () => focus, done: () => done, unlocks,
    content: () => text(tree),
    reject(message: string) { rejection = message },
    async submit() { await elements(tree).find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} }); flush() },
    button: () => elements(tree).find((el) => el.props.type === 'submit')!,
  }
}

for (const file of ['WindowsFirstRun.tsx', 'Onboarding.tsx']) {
  describe(file, () => {
    test('per-field word 1 then word 2 survives deferred dispatch cleanup; all 12 unlock', async () => {
      const ui = setup(file)
      expect(ui.fields()).toHaveLength(12)
      ui.change(0, expectedWords[0], true) // React may eagerly compute the first update.
      expect(ui.values()[0]).toBe('abandon')
      for (let index = 1; index < 12; index++) {
        // Move to the next field, then enter it in a separate dispatch.
        expect(ui.key(index - 1, 'Enter')).toBe(true)
        expect(ui.focus()).toBe(index)
        ui.change(index, expectedWords[index])
        expect(ui.values()[index]).toBe(expectedWords[index])
      }
      expect(ui.values()).toEqual(expectedWords)
      await ui.submit()
      expect(ui.unlocks).toEqual([phrase])
      expect(ui.done()).toBe(1)
    })
    test('incremental typing works even when the first update is deferred', () => {
      const ui = setup(file)
      for (const value of ['a', 'ab', 'aba', 'aban', 'aband', 'abando', 'abandon']) ui.change(0, value)
      expect(ui.values()[0]).toBe('abandon')
      expect(ui.button().props.disabled).toBe(true)
    })
    for (const index of [0, 5, 11]) {
      test(`full phrase pasted into field ${index + 1} fills all 12 words`, async () => {
        const ui = setup(file)
        ui.paste(index, ` \t${expectedWords.join(' \n')}  `)
        expect(ui.values()).toEqual(expectedWords)
        expect(ui.focus()).toBe(11)
        expect(ui.button().props.disabled).toBe(false)
        await ui.submit()
        expect(ui.unlocks).toEqual([phrase])
      })
    }
    test('full phrase received as one input change distributes all words', () => {
      const ui = setup(file)
      ui.change(0, phrase)
      expect(ui.values()).toEqual(expectedWords)
    })
    test('single word and partial paste keep the selected start field', () => {
      const ui = setup(file)
      ui.paste(0, 'abandon')
      expect(ui.focus()).toBe(1)
      ui.paste(1, 'ability able')
      expect(ui.values().slice(0, 4)).toEqual(['abandon', 'ability', 'able', ''])
      expect(ui.focus()).toBe(3)
      ui.paste(11, 'about above')
      expect(ui.values()).toHaveLength(12)
      expect(ui.values()[11]).toBe('about')
      const before = ui.values()
      ui.paste(4, ' \n\t ')
      expect(ui.values()).toEqual(before)
    })
    test('space/Enter advance; Backspace in an empty field goes back without erasing', () => {
      const ui = setup(file)
      ui.paste(0, 'abandon ability')
      expect(ui.key(0, ' ')).toBe(true)
      expect(ui.focus()).toBe(1)
      ui.key(2, 'Backspace')
      expect(ui.focus()).toBe(1)
      expect(ui.values()[1]).toBe('ability')
      ui.key(1, 'Backspace')
      expect(ui.focus()).toBe(1) // Browser handles ordinary deletion.
      ui.change(1, '')
      ui.key(1, 'Backspace')
      expect(ui.focus()).toBe(0)
      ui.change(0, '')
      ui.key(0, 'Backspace')
      expect(ui.focus()).toBe(0)
      ui.key(11, 'Enter')
      expect(ui.focus()).toBe(11)
    })
    test('incomplete phrase does not invoke unlock', async () => {
      const ui = setup(file)
      ui.paste(0, 'abandon')
      await ui.submit()
      expect(ui.unlocks).toHaveLength(0)
      expect(ui.done()).toBe(0)
    })
    test('wrong word stays inline; editing clears error and allows retry', async () => {
      const ui = setup(file)
      ui.reject('Invalid recovery word: check your phrase.')
      ui.paste(0, phrase.replace('about', 'notaword'))
      await ui.submit()
      expect(ui.done()).toBe(0)
      expect(ui.content()).toContain('Invalid recovery word: check your phrase.')
      expect(ui.fields()).toHaveLength(12)
      ui.change(11, 'about')
      expect(ui.content()).not.toContain('Invalid recovery word: check your phrase.')
      ui.reject('')
      await ui.submit()
      expect(ui.unlocks).toEqual([phrase.replace('about', 'notaword'), phrase])
      expect(ui.done()).toBe(1)
    })
  })
}


describe('onboarding error boundary', () => {
  for (const name of ['WindowsFirstRun', 'Onboarding']) {
    test(`${name} places its entire view below the boundary`, () => {
      const source = readFileSync(new URL(`../src/${name}.tsx`, import.meta.url), 'utf8')
      const View = () => null
      const Root = component(source, name, {
        React: { createElement }, OnboardingErrorBoundary, [`${name}View`]: View,
      })
      const tree = Root()
      expect(tree.type).toBe(OnboardingErrorBoundary)
      expect(tree.props.children.type).toBe(View)
    })
  }
  test('failure renders an honest alert; retry remounts children without exposing input', () => {
    const child = createElement('div', null, 'setup form')
    const boundary = new OnboardingErrorBoundary({ children: child })
    expect(boundary.render()).toBe(child)
    // Exercise the lifecycle state transition directly; actual React capture is
    // explicitly part of the native/browser gate, not claimed by this unit test.
    boundary.state = OnboardingErrorBoundary.getDerivedStateFromError()
    const html = renderToStaticMarkup(boundary.render())
    expect(html).toContain('role="alert"')
    expect(html).toContain('Setup couldn’t continue')
    expect(html).toContain('unexpected error')
    expect(html).toContain('re-enter your recovery phrase')
    expect(html).not.toContain('setup form')
    expect(html).not.toContain(phrase)
    boundary.setState = (state: any) => { boundary.state = state }
    const retry = elements(boundary.render()).find((el) => el.type === 'button')!
    expect(text(retry)).toBe('Try setup again')
    retry.props.onClick()
    expect(boundary.render()).toBe(child)
  })
})
