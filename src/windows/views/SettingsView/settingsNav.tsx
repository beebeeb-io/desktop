/**
 * The settings navigation rail. Sections come from `settingsNavigation` and are filtered by
 * the capabilities the current build reports, so a route never appears where it cannot work.
 */
import { useCapabilities, supportsRoute } from '../../../capabilities'
import { availableSettingsSections, type SettingsNavId } from '../../settingsNavigation'
import { usePlatform } from '../../../platform'
import { T, NavIcon } from '../../ui'

export function SettingsNav({
  activeNav,
  loggedIn,
  onChange,
}: {
  activeNav: SettingsNavId
  loggedIn: boolean
  onChange: (id: SettingsNavId) => void
}) {
  // `null` while the real host platform hasn't resolved yet — passed through
  // as-is so `availableSettingsSections` renders the neutral "Shell
  // integration" label instead of a guessed one that then flips (PR #34 review).
  const caps = useCapabilities()
  const { name: platform, resolved } = usePlatform()
  const sections = availableSettingsSections(loggedIn, resolved ? platform : null).map((section) => ({ ...section, items: section.items.filter((item) => supportsRoute(caps, item.id)) })).filter((section) => section.items.length > 0)

  return (
    <div style={{ background: T.paper2, borderRight: `1px solid ${T.line}`, padding: '16px 10px', overflow: 'auto', display: 'flex', flexDirection: 'column', minWidth: 0 }}>
      <div style={{ padding: '4px 8px 10px' }}>
        <div style={{ fontSize: 13, fontWeight: 700, color: T.ink, marginBottom: 3 }}>Settings</div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4 }}>Device, sync, updates, and notifications.</div>
      </div>

      {sections.map((section) => (
        <div key={section.heading} style={{ marginBottom: 10 }}>
          <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, padding: '4px 10px', marginBottom: 2 }}>
            {section.heading}
          </div>
          {section.items.map((item) => {
            const active = item.id === activeNav
            return (
              <button
                key={item.id}
                onClick={() => onChange(item.id)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  width: '100%',
                  padding: '7px 10px',
                  borderRadius: 6,
                  fontSize: 12.5,
                  fontWeight: active ? 600 : 400,
                  color: active ? T.ink : T.ink2,
                  background: active ? T.paper : 'transparent',
                  border: active ? `1px solid ${T.line}` : '1px solid transparent',
                  borderLeft: active ? `3px solid ${T.amberDeep}` : '3px solid transparent',
                  cursor: 'pointer',
                  textAlign: 'left' as const,
                  fontFamily: T.fontSans,
                  marginBottom: 2,
                  transition: 'background 100ms',
                }}
              >
                <NavIcon name={item.icon} size={12} color={active ? T.amberDeep : T.ink3} />
                {item.label}
              </button>
            )
          })}
        </div>
      ))}
    </div>
  )
}
