/**
 * The census engine (tests/fixtures/finderResultCensus.ts) proved on source snippets BEFORE it is
 * trusted on the real tree (task 17b, fix round 1): every leak shape it claims to catch is shown
 * failing here, and every guard it claims to understand is shown excluding a use. The real tree is
 * checked in tests/finderSetupSourceContract.test.ts.
 */
import { describe, expect, test } from 'bun:test'
import { censusOfSource, familyLiterals, type Site } from './fixtures/finderResultCensus'

const sitesOf = (code: string, file = 'x.tsx'): Site[] => censusOfSource(file, code)
const macLeaks = (code: string) => sitesOf(code).flatMap((site) => site.violations.filter((v) => v.macReachable).map((v) => v.kind))
const allLeaks = (code: string) => sitesOf(code).flatMap((site) => site.violations.map((v) => `${v.macReachable ? 'mac' : 'nonmac'}:${v.kind}`))

const wrap = (body: string, command = 'open_in_finder') => `
  async function handler() {
    const result = await command<void>('${command}')
    ${body}
  }
`

describe('the census engine finds every leak shape on a Mac', () => {
  const leaks: Array<[string, string, string]> = [
    ['a property read', 'if (!result.ok) showToast({ message: result.reason })', 'reason'],
    ['an element read', "if (!result.ok) showToast({ message: result['reason'] })", 'reason'],
    ['a computed element read', 'const key = pick(); if (!result.ok) showToast({ message: result[key] })', 'computed'],
    ['a destructured reason', 'const { reason } = result', 'destructure'],
    ['a rest element', 'const { ok, ...rest } = result', 'destructure'],
    ['a destructured warnings', 'const { warnings } = result.value', 'destructure'],
    ['a nested destructured warnings', 'const { value: { warnings } } = result', 'destructure'],
    ['an alias', 'const r = result; show(r)', 'alias'],
    ['a spread into an object', 'show({ ...result })', 'passed-on'],
    ['a hand-off to an unknown function', 'show(result)', 'passed-on'],
    ['the reason through an optional chain', 'show(result?.reason)', 'reason'],
  ]
  for (const [name, body, kind] of leaks) {
    test(name, () => {
      expect(macLeaks(wrap(body))).toContain(kind)
    })
  }

  test('a repair\'s warnings, read or handed on, when the command is reset_macos_integration', () => {
    expect(macLeaks(wrap('show(result.value.warnings.join(" "))', 'reset_macos_integration'))).toContain('warnings')
    expect(macLeaks(wrap('if (result.ok) show(result.value)', 'reset_macos_integration'))).toContain('passed-on')
  })

  test('a result read inside a .then callback', () => {
    expect(macLeaks(`command('open_in_finder').then((r) => { if (!r.ok) showToast({ message: r.reason }) })`)).toEqual(['reason'])
  })

  test('a destructuring in the declaration itself', () => {
    expect(macLeaks(`async function f() { const { ok, reason } = await command('open_in_finder') }`)).toEqual(['destructure'])
  })

  test('a result that is passed straight to something, which cannot be followed', () => {
    expect(macLeaks(`function f() { show(await command('open_in_finder')) }`)).toEqual(['unanalysed'])
  })

  test('the wrappers and finder.run are call sites too', () => {
    const code = `
      async function a() { const r = await loadFinderSetup(); show(r.reason) }
      async function b() { const r = await finder.run(action); show(r.reason) }
      async function c() { const r = await copyFinderSetupDetails(); show(r.reason) }
    `
    expect(sitesOf(code).map((s) => s.family)).toEqual(['wrapper:loadFinderSetup', 'finder.run', 'wrapper:copyFinderSetupDetails'])
    expect(macLeaks(code)).toEqual(['reason', 'reason', 'reason'])
  })
})

describe('the census engine passes what is safe, and understands the guards of this codebase', () => {
  test('.ok, .unsupported and a success value that carries nothing free-text', () => {
    expect(allLeaks(wrap('if (!result.ok) return; setCount(result.value.pending_operations_preserved)', 'reset_macos_integration'))).toEqual([])
    expect(allLeaks(wrap('if (!result.ok) { flag(result.unsupported); return }'))).toEqual([])
  })

  test('a result returned, and a sanitiser given the result or its value', () => {
    expect(allLeaks(`async function f() { const result = await command('open_in_finder'); return result }`)).toEqual([])
    expect(allLeaks(wrap('if (result.ok) show(repairNote(result.value))', 'reset_macos_integration'))).toEqual([])
    expect(allLeaks(wrap('if (result.ok) show(finderRepairWarningNote(result.value))', 'reset_macos_integration'))).toEqual([])
  })

  test('a reason on the non-macOS side of a ternary is seen and excluded, in every spelling of the guard', () => {
    for (const guard of ["platform === 'macos'", "'macos' === platform", 'isMacos', 'isMac', 'mac']) {
      expect(allLeaks(wrap(`if (!result.ok) show(${guard} ? sentence() : result.reason)`))).toEqual(['nonmac:reason'])
    }
    for (const guard of ["platform !== 'macos'", '!isMacos', "!(platform === 'macos')"]) {
      expect(allLeaks(wrap(`if (!result.ok) show(${guard} ? result.reason : sentence())`))).toEqual(['nonmac:reason'])
    }
  })

  test('the non-macOS side of an if/else and of a && chain', () => {
    expect(allLeaks(wrap("if (!result.ok) { if (platform === 'macos') { show(sentence()) } else { show(result.reason) } }"))).toEqual(['nonmac:reason'])
    expect(allLeaks(wrap("if (!result.ok) { if (platform !== 'macos') { show(result.reason) } }"))).toEqual(['nonmac:reason'])
    expect(allLeaks(wrap('if (!result.ok) { !isMacos && show(result.reason) }'))).toEqual(['nonmac:reason'])
    expect(allLeaks(wrap("if (!result.ok) { !isMacos && ready && show(result.reason) }"))).toEqual(['nonmac:reason'])
  })

  test('nested: a warnings read under a non-macOS arm inside a macOS-guarded expression', () => {
    const code = wrap("const note = platform === 'macos' ? null : result.value.warnings.length > 0 ? result.value.warnings.join(' ') : null", 'reset_macos_integration')
    expect(allLeaks(code)).toEqual(['nonmac:warnings', 'nonmac:warnings'])
  })
})

describe('the census engine fails closed when it cannot tell', () => {
  test('the macOS side of a guard is reachable on a Mac', () => {
    expect(macLeaks(wrap("if (!result.ok) show(platform === 'macos' ? result.reason : sentence())"))).toEqual(['reason'])
    expect(macLeaks(wrap('if (!result.ok) { if (isMacos) { show(result.reason) } }'))).toEqual(['reason'])
  })

  test('a guard it does not recognise is not a guard', () => {
    expect(macLeaks(wrap('if (!result.ok) show(flag ? sentence() : result.reason)'))).toEqual(['reason'])
    expect(macLeaks(wrap("if (!result.ok) show(os.name === 'windows' ? result.reason : sentence())"))).toEqual(['reason'])
  })

  test('&& with a macOS guard on the left is the macOS side', () => {
    expect(macLeaks(wrap('if (!result.ok) { isMacos && show(result.reason) }'))).toEqual(['reason'])
  })

  test('an || with a not-macOS guard on the left is the macOS side', () => {
    expect(macLeaks(wrap('if (!result.ok) { !isMacos || show(result.reason) }'))).toEqual(['reason'])
  })
})

describe('the literal census sees a command reached some other way', () => {
  test('a name held in a variable shows as a literal with no call site', () => {
    const code = `const name = 'open_in_finder'\nfunction f() { return command(name) }`
    expect(sitesOf(code)).toEqual([])
    expect(familyLiterals('x.ts', code)).toEqual({ open_in_finder: 1 })
  })

  test('calls, labels and tables each count once, comments never', () => {
    const code = `
      // open_in_finder in a comment
      const a = command('open_in_finder')
      const b = commandUnavailableLabel('open_in_finder')
      const c = { show: 'finder_setup_show_app', retry: \`finder_setup_retry\` }
    `
    expect(familyLiterals('x.ts', code)).toEqual({ open_in_finder: 2, 'finder_setup_*': 2 })
  })
})
