// General › Storage location (DW-0788 family): where new files are stored, inside the EU. Countries only.
import { useEffect, useRef, useState } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { Btn, Group, Page, Row, Switch } from '../shell/kit'
import { PLACES } from '../../../source/fixtures/settings-today'
import { BackLink, ErrorBanner, InfoBanner, Toast } from './ui'

export interface LocationProps { chosen?: string; strict?: boolean; unavailable?: boolean; error?: boolean; toast?: boolean }

export function Location({ chosen: c0 = 'default', strict: s0 = false, unavailable, error, toast: toast0 }: LocationProps = {}) {
  const [chosen, setChosen] = useState(c0)
  const [strict, setStrict] = useState(s0)
  const [failed, setFailed] = useState(!!error)
  const [toast, setToast] = useState(!!toast0)
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)
  useEffect(() => () => clearTimeout(timer.current), [])
  const pick = (id: string) => { setChosen(id); setToast(true); clearTimeout(timer.current); timer.current = setTimeout(() => setToast(false), 2500) }
  const place = PLACES.find((p) => p.id === chosen)
  const name = place?.label ?? 'Europe'
  return (
    <Page title="Storage location" sub="Choose where new files are stored, inside the EU." back={<BackLink to="general">General</BackLink>}>
      {failed ? (
        <ErrorBanner title="Couldn't load storage locations" action={<Btn onClick={() => setFailed(false)}>Try again</Btn>}>Your storage location is unchanged. Files stay where they are.</ErrorBanner>
      ) : (
        <>
          {unavailable ? (
            <InfoBanner title="Your chosen location isn't available right now.">
              {strict ? `Uploads wait until ${name} can take them.` : 'New files go wherever Beebeeb places them by default.'}
            </InfoBanner>
          ) : null}
          <Group note="Existing files stay where they are. Only new files use the location you choose.">
            <div role="radiogroup" aria-label="Storage location" className="divide-y divide-line">
              {PLACES.map((p) => (
                <button key={p.id} type="button" role="radio" aria-checked={chosen === p.id} aria-disabled={!p.live} disabled={!p.live} onClick={() => pick(p.id)}
                  className={`flex min-h-[44px] w-full items-center gap-3 px-4 py-2 text-left ${p.live ? 'hover:bg-paper-2' : 'opacity-50'}`}>
                  <span className="w-4">{chosen === p.id ? <Icon name="check" size={14} /> : null}</span>
                  <span className="min-w-0 flex-1"><span className="block text-[13px] text-ink">{p.label}</span>{p.sub ? <span className="block text-[12px] text-ink-3">{p.sub}</span> : null}</span>
                </button>
              ))}
            </div>
          </Group>
          {chosen !== 'default' ? (
            <Group>
              <Row label={`Only store in ${name}`} sub={`Off: it goes to another EU country. On: the upload stops and waits.`}
                right={<Switch label={`Only store in ${name}`} on={strict} onChange={setStrict} />} />
            </Group>
          ) : null}
          {chosen !== 'default' ? <p className="-mt-3 px-1 text-[12px] text-ink-3">If {name} can't take a new file.</p> : null}
        </>
      )}
      {toast ? <Toast>Storage location updated</Toast> : null}
    </Page>
  )
}
