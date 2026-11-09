// Devices and sessions (DW-A55) and the confirm before a device is signed out (DW-A56).
import { useEffect, useState } from 'react'
import { Dialog, Group, Page } from '../shell/kit'
import { useAccount, run, errorText, type Scenario } from './data'
import type { DeviceGroup } from './types'
import { Btn, Pill, relative } from './ui'

type Confirm = { kind: 'one'; device: DeviceGroup } | { kind: 'others' }

export function DevicesPane({ scenario = 'default', confirm: initialConfirm }: { scenario?: Scenario; confirm?: string } = {}) {
  const { state, setState, reload } = useAccount(scenario)
  const [confirm, setConfirm] = useState<Confirm | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!initialConfirm) return
    const device = state.devices.find((d) => d.name === initialConfirm)
    if (device) setConfirm({ kind: 'one', device })
  }, [initialConfirm, state.devices])

  const signOut = async (c: Confirm) => {
    setConfirm(null)
    const ids = c.kind === 'one' ? c.device.session_ids : []
    const results = c.kind === 'one'
      ? await Promise.all(ids.map((sessionId) => run('account_revoke_session', { sessionId }, null)))
      : [await run('account_revoke_other_sessions', {}, { revoked: 3 })]
    const failed = results.find((r) => !r.ok)
    if (failed && !failed.ok) setError(errorText(failed.code))
    setState((s) => ({ ...s, devices: s.devices.filter((d) => (c.kind === 'one' ? d !== c.device : d.is_current)) }))
    void reload()
  }

  const others = state.devices.filter((d) => !d.is_current).length
  return (
    <Page title="Devices and sessions" sub="Every device signed in to your Beebeeb account, in every app."
      action={others ? <Btn variant="danger" onClick={() => setConfirm({ kind: 'others' })}>Sign out everywhere else</Btn> : undefined}>
      {error ? <p role="alert" className="text-[12px] text-red">{error}</p> : null}
      <Group note="A device signed out here stops working the next time it contacts Beebeeb. Signing out of the apps on this PC is on Sign-in and security.">
        {state.devices.map((d) => (
          <div key={d.session_ids.join()} className="flex min-h-[44px] items-center gap-3 px-4 py-2" data-testid="device-row"
            aria-label={`${d.name}, ${d.apps.join(' and ')}, ${relative(d.last_active_at)}`}>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-[13px] leading-5 text-ink">{d.name}{d.is_current ? <Pill tone="green">This device</Pill> : null}</div>
              <div className="text-[12px] leading-4 text-ink-3">{d.os} · {d.apps.join(', ')} · {relative(d.last_active_at)}</div>
            </div>
            {d.country_code ? <span className="font-mono text-[12px] text-ink-3">{d.country_code}</span> : null}
            {d.is_current ? null : <Btn onClick={() => setConfirm({ kind: 'one', device: d })} aria-label={`Sign out ${d.name}`}>Sign out</Btn>}
          </div>
        ))}
      </Group>
      {confirm ? <ConfirmSignOut confirm={confirm} onCancel={() => setConfirm(null)} onConfirm={() => void signOut(confirm)} /> : null}
    </Page>
  )
}

function ConfirmSignOut({ confirm, onCancel, onConfirm }: { confirm: Confirm; onCancel: () => void; onConfirm: () => void }) {
  const title = confirm.kind === 'one' ? `Sign out ${confirm.device.name}?` : 'Sign out everywhere else?'
  return (
    <Dialog labelId="signout-device-h" title={title} onCancel={onCancel}
      actions={<><Btn onClick={onCancel}>Cancel</Btn>
        <Btn variant="danger" onClick={onConfirm} aria-label={confirm.kind === 'one' ? `Sign out ${confirm.device.name}` : 'Sign out every other device'}>Sign out</Btn></>}>
      <p>{confirm.kind === 'one'
        ? `The Beebeeb apps on ${confirm.device.name} are signed out when they next contact Beebeeb.`
        : 'Every device except this PC is signed out when it next contacts Beebeeb.'}</p>
      <p className="mt-2">The data on {confirm.kind === 'one' ? 'that device' : 'those devices'} stays until it is removed there. You get a notice in the app that a device was signed out.</p>
    </Dialog>
  )
}
