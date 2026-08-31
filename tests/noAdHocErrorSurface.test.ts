/**
 * Tests for the `no-ad-hoc-error-surface` lint rule (task 1319).
 *
 * The rule exists because four successive hand enumerations of the desktop error
 * states all undercounted. A guard nobody has tested is just a fifth opinion, so the
 * cases below pin the two classification bugs found while building it — both of which
 * produced WRONG answers in the dangerous direction (calling a legitimate inline error
 * a straggler, which would tell someone to delete a control or break a load state).
 */
import { describe, it } from 'bun:test'
import { RuleTester } from 'eslint'
import tsParser from '@typescript-eslint/parser'
import rule from '../eslint-rules/no-ad-hoc-error-surface.mjs'

RuleTester.describe = describe
RuleTester.it = it

const ruleTester = new RuleTester({
  languageOptions: {
    parser: tsParser,
    ecmaVersion: 'latest',
    sourceType: 'module',
    parserOptions: { ecmaFeatures: { jsx: true } },
  },
})

const straggler = `
  function Panel() {
    const [message, setMessage] = useState(null)
    const install = async () => {
      const result = await command('install_thing')
      if (!result.ok) setMessage(result.reason)
    }
    return <div>{message && <Notice>{message}</Notice>}<button onClick={install}>Go</button></div>
  }
`

ruleTester.run('no-ad-hoc-error-surface', rule, {
  valid: [
    {
      name: 'error that GATES a control stays inline (the FinderInstallStep shape)',
      // Regression test: the rule originally read the INNERMOST `&&`, saw `!isMacos`
      // instead of the <button>, and wrongly called this a straggler.
      code: `
        function Panel() {
          const [message, setMessage] = useState(null)
          const pick = async () => {
            const r = await command('pick')
            if (!r.ok) setMessage(r.reason)
          }
          return <div>
            {message && <div className="notice">{message}</div>}
            {message && !isMacos && <button onClick={skip}>Continue without install</button>}
          </div>
        }
      `,
    },
    {
      name: 'LOAD failure set directly inside useEffect stays inline',
      code: `
        function Panel() {
          const [error, setError] = useState(null)
          useEffect(() => {
            command('list').then((r) => { if (!r.ok) setError(r.reason) })
          }, [])
          return <div>{error && <ErrorBlock reason={error} />}</div>
        }
      `,
    },
    {
      name: 'LOAD failure via a named load() called from useEffect stays inline',
      // Regression test: reading only lexical nesting missed this idiom and classified
      // TrashView's list-load failure as a transient action.
      code: `
        function Panel() {
          const [error, setError] = useState(null)
          const load = async () => {
            const result = await command('list')
            if (!result.ok) setError(result.reason)
          }
          useEffect(() => { void load() }, [])
          return <div>{error && <ErrorBlock reason={error} onRetry={load} />}</div>
        }
      `,
    },
    {
      name: 'state that never holds a command failure is not an error state',
      code: `
        function Panel() {
          const [syncRoot, setSyncRoot] = useState(null)
          const pick = async () => {
            const r = await command('pick')
            if (r.ok) setSyncRoot(r.value)
          }
          return <div>{syncRoot && <div>{syncRoot}</div>}</div>
        }
      `,
    },
    {
      name: 'action failure already routed to a toast is not rendered inline',
      code: `
        function Panel() {
          const { showToast } = useToast()
          const save = async () => {
            const r = await command('save')
            if (!r.ok) showToast({ variant: 'error', title: 'Nope', message: r.reason })
          }
          return <button onClick={save}>Save</button>
        }
      `,
    },
  ],

  invalid: [
    {
      name: 'transient action failure rendered inline is a straggler',
      code: straggler,
      errors: [{ messageId: 'transientActionInline' }],
    },
    {
      name: 'transient action failure prop-drilled into another component is a straggler',
      // The SettingsView shell `notice` shape: never rendered here, handed to a child.
      code: `
        function Shell() {
          const [notice, setNotice] = useState(null)
          const change = async (patch) => {
            const result = await command('set_config', { patch })
            if (!result.ok) setNotice(result.reason)
          }
          return <SyncPanel notice={notice} onChange={change} />
        }
      `,
      errors: [{ messageId: 'transientActionInline' }],
    },
  ],
})
