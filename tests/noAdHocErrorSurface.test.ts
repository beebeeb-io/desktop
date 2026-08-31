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
      name: 'CORRECTION-BLOCKING: sign-in error beside the form the user retypes stays inline',
      // The pwError/totpError shape. The rule originally called these stragglers; checking
      // the source showed the PRINCIPLE was missing a clause, not that the code was wrong.
      code: `
        function Panel() {
          const [pwError, setPwError] = useState(null)
          const submit = async (e) => {
            e.preventDefault()
            const r = await desktopLogin(email, password)
            if (!r.ok) setPwError(r.reason)
          }
          return <div>
            {pwError && <Notice kind="error">{pwError}</Notice>}
            <form onSubmit={submit}>
              <input type="email" value={email} onChange={onEmail} />
              <input type="password" value={password} onChange={onPassword} />
            </form>
          </div>
        }
      `,
    },
    {
      name: 'CORRECTION-BLOCKING: modal password error beside its input stays inline',
      // TrashView's deleteError — no <form>, the error div is a direct sibling of the input.
      code: `
        function Panel() {
          const [deleteError, setDeleteError] = useState(null)
          const del = async () => {
            const r = await command('delete', { password })
            if (!r.ok) setDeleteError(r.reason)
          }
          return <Card>
            <input type="password" value={password} onChange={onPassword} />
            {deleteError && <div>{deleteError}</div>}
            <GhostButton onClick={del}>Delete</GhostButton>
          </Card>
        }
      `,
    },
    {
      name: 'SUBSTITUTE RENDER: an error returned AS the view body stays inline',
      // DesktopVersionHistory's versionNotice. loadVersions is a load triggered by a click
      // rather than by mount, so isLoadPath cannot see it; the error is returned as the
      // whole panel body. Toasting it falls through to "No earlier versions", making a load
      // failure indistinguishable from an empty result — the TrashView bug from 1255.
      code: `
        function Panel() {
          const [versionNotice, setVersionNotice] = useState(null)
          const loadVersions = useCallback(async (file) => {
            const r = await desktopListFileVersions(file.id)
            if (!r.ok) setVersionNotice(r.reason)
          }, [])
          const body = () => {
            if (loading) return <div className="empty">Loading…</div>
            if (versionNotice != null) return <div className="notice">{versionNotice}</div>
            return <div className="empty">No earlier versions</div>
          }
          return <div onClick={loadVersions}>{body()}</div>
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
    {
      name: 'CommandResult parked in state that GATES a control stays inline (UnlockStep)',
      code: `
        function Panel() {
          const [result, setResult] = useState<CommandResult<void> | null>(null)
          const unlock = async () => { setResult(await command('unlock')) }
          return <div>
            {result && !result.ok && <div className="notice">{result.reason}</div>}
            {result && !result.ok && result.unsupported && <button onClick={onDone}>Continue</button>}
          </div>
        }
      `,
    },
  ],

  invalid: [
    {
      name: 'CommandResult parked in state, action-only, gating nothing, is a straggler',
      // Found by diffing the rule's census against a hand count: reading only setter
      // arguments missed this shape entirely, because `.reason` is read at the render site.
      code: `
        function Panel() {
          const [result, setResult] = useState<CommandResult<void> | null>(null)
          const save = async () => { setResult(await command('save')) }
          return <div>
            <button onClick={save}>Save</button>
            {result && !result.ok && <div className="notice">{result.reason}</div>}
          </div>
        }
      `,
      errors: [{ messageId: 'transientActionInline' }],
    },
    {
      name: 'a toggle failure is NOT correction-blocking just because the page has inputs',
      // Guards against the third clause becoming a blanket exemption: the error renders
      // among toggles, in a different subtree from the unrelated search box.
      code: `
        function Panel() {
          const [saveError, setSaveError] = useState(null)
          const toggle = async () => {
            const r = await command('save_pref')
            if (!r.ok) setSaveError(r.reason)
          }
          return <div>
            <div><input type="search" value={q} onChange={onQ} /></div>
            <Card>
              <Toggle onChange={toggle} label="Notify me" />
              {saveError && <div>{saveError}</div>}
            </Card>
          </div>
        }
      `,
      errors: [{ messageId: 'transientActionInline' }],
    },
    {
      name: 'early returns elsewhere do NOT exempt an error annotating the main body',
      // Guards the substitute-render clause against becoming a blanket exemption: this
      // component has two JSX returns, but the error is buried in a large returned tree,
      // which makes it an annotation rather than a replacement for the view.
      code: `
        function Panel() {
          const [applyError, setApplyError] = useState(null)
          const apply = async () => {
            const r = await command('apply')
            if (!r.ok) setApplyError(r.reason)
          }
          if (loading) return <div className="skeleton">Loading…</div>
          return (
            <div>
              <h1>Selective sync</h1>
              <p>Choose folders.</p>
              <button onClick={apply}>Apply</button>
              {applyError && <div>{applyError}</div>}
            </div>
          )
        }
      `,
      errors: [{ messageId: 'transientActionInline' }],
    },
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
