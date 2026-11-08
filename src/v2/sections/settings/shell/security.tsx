// Security (DW-0716, DW-0716b): one pane. Drive's App lock sits above the shared Sign-in rows (the kit's
// securityExtra slot, DW-A52). Turning the lock off or changing the delay asks for Windows Hello first.
import { useState } from 'react'
import { Btn, Choice, Dialog, Group, Page, Row, Switch } from './kit'

type Delay = 'immediate' | '5' | '30' | '60'
const DELAYS: { value: Delay; label: string }[] = [
  { value: 'immediate', label: 'Immediately' }, { value: '5', label: '5 min' }, { value: '30', label: '30 min' }, { value: '60', label: '60 min' },
]

export function Security({ lockOn = true, popupOpen }: { lockOn?: boolean; popupOpen?: boolean } = {}) {
  const [on, setOn] = useState(lockOn)
  const [delay, setDelay] = useState<Delay>('5')
  const [hello, setHello] = useState<null | (() => void)>(null)
  const ask = (apply: () => void) => setHello(() => apply)
  return (
    <Page title="Sign-in and security" action={<Btn onClick={() => undefined}>Security checkup · 7 of 7</Btn>}>
      <Group title="App lock" note={on ? 'Locks Beebeeb’s screens only. Sync keeps running while it is locked, and File Explorer follows the PC’s own lock.' : undefined}>
        <Row label="Lock Beebeeb" sub={on ? 'Windows Hello · locks again after' : 'Off: anyone using this PC can open Beebeeb'}
          right={<>
            {on ? <Choice label="Lock again after" value={delay} options={DELAYS} open={popupOpen} onPick={(d) => ask(() => setDelay(d))} /> : null}
            <Switch label="Lock Beebeeb" on={on} onChange={(v) => ask(() => setOn(v))} />
          </>} />
      </Group>
      <Group title="Sign-in">
        <Row label="Password" value="Changed 3 months ago" onClick={() => undefined} />
        <Row label="Two-factor" value="Authenticator app" onClick={() => undefined} />
        <Row label="Passkeys" value="2" onClick={() => undefined} />
        <Row label="Sign-in activity" onClick={() => undefined} />
      </Group>
      <Group title="Devices">
        <Row label="Devices and sessions" value="4" onClick={() => undefined} />
      </Group>
      <Group title="Sign out and delete">
        <Row label="Sign out of this PC" onClick={() => undefined} />
        <Row label="Delete account" onClick={() => undefined} />
      </Group>
      {hello ? (
        <Dialog labelId="hello-h" title="Confirm with Windows Hello" onCancel={() => setHello(null)}
          actions={<><Btn onClick={() => setHello(null)}>Cancel</Btn><Btn variant="amber" onClick={() => { hello(); setHello(null) }}>Continue</Btn></>}>
          Windows Hello confirms it is you before the lock changes.
        </Dialog>
      ) : null}
    </Page>
  )
}
