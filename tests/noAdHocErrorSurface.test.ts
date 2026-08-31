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
      name: 'SPLIT NEGATIVE: a PURE load state is still fully exempt, not a split candidate',
      code: `
        function Panel() {
          const [notice, setNotice] = useState(null)
          const load = async () => {
            const r = await command('list')
            if (!r.ok) setNotice(r.reason)
          }
          useEffect(() => { void load() }, [])
          return <div>{notice && <div className="notice">{notice}</div>}</div>
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
      name: 'LOAD reached only via an onRetry callback is still a load, not a split',
      // Task 1330. ActivityView and SettingsView wire `load` as <ErrorBlock onRetry={load}/>
      // and never call it from a useEffect, so isLoadPath called it an action and the rule
      // reported a load+action split. Toasting that half would have removed the very
      // ErrorBlock whose Retry button calls it.
      code: `
        function Panel() {
          const [err, setErr] = useState(null)
          const load = () => {
            void (async () => {
              const r = await accountActivityFeed(1, 50)
              if (!r.ok) setErr({ reason: r.reason, unsupported: r.unsupported })
            })()
          }
          if (err) return <ErrorBlock reason={err.reason} onRetry={load} />
          return <div>ok</div>
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
      name: 'CLAUSE 4: error rendered only inside a sibling machine error phase stays inline',
      code: `
        function Panel() {
          const [phase, setPhase] = useState('idle')
          const [errorMsg, setErrorMsg] = useState('')
          const confirm = async () => {
            const r = await clearSession()
            if (!r.ok) { setErrorMsg(r.reason); setPhase('error') }
          }
          return <div>
            <button onClick={confirm}>Disconnect</button>
            {phase === 'error' && (
              <div><div>{errorMsg}</div><button onClick={confirm}>Retry</button></div>
            )}
          </div>
        }
      `,
    },
    {
      name: 'CLAUSE 4: the ternary shape (downgradeInstallError) stays inline',
      code: `
        function Panel() {
          const [downgradeInstallState, setS] = useState('idle')
          const [downgradeInstallError, setE] = useState(null)
          const go = async () => {
            const r = await command('downgrade')
            if (!r.ok) { setE(r.reason); setS('error') }
          }
          return <div>
            <button onClick={go}>Downgrade</button>
            <div>{downgradeInstallState === 'error' && downgradeInstallError
              ? \`Downgrade failed: \${downgradeInstallError}\`
              : 'Read the release notes first.'}</div>
          </div>
        }
      `,
    },
    {
      name: 'CLAUSE 4: gate written through a local boolean (UpdateBanner) stays inline',
      code: `
        function Panel() {
          const [installState, setState] = useState('idle')
          const [installError, setErr] = useState(null)
          const install = async () => {
            const r = await command('install')
            if (!r.ok) { setErr(r.reason); setState('error') }
          }
          const installFailed = installState === 'error' && installError != null
          return <div>
            <button onClick={install}>Install</button>
            {installFailed && <span>Install failed: {installError}</span>}
          </div>
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
      name: 'SPLIT CANDIDATE: a state carrying BOTH a load and an action failure is reported',
      // The PinningStep / SelectiveSync shape. The rule used to exempt this wholesale
      // because ONE setter sat on a load path — a silent under-report, which is the
      // failure mode this whole guard exists to end.
      code: `
        function Panel() {
          const [notice, setNotice] = useState(null)
          useEffect(() => {
            command('list_remote_tree').then((tree) => {
              if (!tree.ok) setNotice(tree.reason)
            })
          }, [])
          const toggle = async (item) => {
            const r = await command('set_recursive_pin', { id: item.id })
            if (!r.ok) setNotice(r.reason)
          }
          return <div>
            {notice && <div className="notice">{notice}</div>}
            <button onClick={toggle}>Toggle</button>
          </div>
        }
      `,
      errors: [{ messageId: 'splitCandidate' }],
      name: 'CLAUSE 4 NEGATIVE: one render OUTSIDE the gate and it is still reported',
      // Condition 2 from the lead's adoption. A component that merely CONTAINS a state
      // machine must not exempt every error in its body — the same over-broad shape
      // rejected for clause 3 when "contains an input anywhere" was refused.
      code: `
        function Panel() {
          const [phase, setPhase] = useState('idle')
          const [errorMsg, setErrorMsg] = useState('')
          const go = async () => {
            const r = await command('go')
            if (!r.ok) { setErrorMsg(r.reason); setPhase('error') }
          }
          return <div>
            <button onClick={go}>Go</button>
            <footer>{errorMsg}</footer>
            {phase === 'error' && <div>{errorMsg}</div>}
          </div>
        }
      `,
      errors: [{ messageId: 'transientActionInline' }],
    },
    {
      name: 'CLAUSE 4 NEGATIVE: a self-gated machine with other phases is NOT exempt',
      // BandwidthView's `st`. It gates on ITSELF, not a sibling, and renders three
      // non-error phases besides. Named as a fourth instance; on inspection it is a view
      // state machine that happens to carry a reason, which is a different shape.
      code: `
        function Panel() {
          const [st, setSt] = useState({ phase: 'idle' })
          const run = async () => {
            const r = await command('speedtest')
            if (!r.ok) setSt({ phase: 'error', reason: r.reason })
          }
          return <div>
            <button onClick={run}>Run</button>
            {st.phase === 'idle' && <div>Idle</div>}
            {st.phase === 'running' && <div>Running…</div>}
            {st.phase === 'error' && <div>{st.reason}</div>}
          </div>
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
