// DW-0515 from the LIVE Shared section, not the frame: the window's Tauri IPC is a recording fake whose
// `share_people` and `share_invite` answer the way the Rust commands do (`share_v2::safety`), so the alert comes from
// a refused seal (`{status: "key-changed"}`) and from a People row whose share was sealed to another key
// (`keyChanged`). Each check is a COUNT, written to shots.log; `--plant` answers as if nothing changed, and every
// count must then fail (the instrument proves it can fail). 2d-2f: the ways out of a refused invite's comparison.
//
//   bun scripts/v2-shots/dw0515.mjs --base http://localhost:5226 --out <dir> [--plant]
import { chromium } from 'playwright'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => {
    if (a.startsWith('--')) acc.push([a.slice(2), all[i + 1] && !all[i + 1].startsWith('--') ? all[i + 1] : 'true'])
    return acc
  }, []),
)
const BASE = args.base ?? 'http://localhost:5226'
const OUT = args.out ?? 'dw0515-shots'
const PLANT = args.plant === 'true'
mkdirSync(OUT, { recursive: true })

const INVITE = '6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01'
const lines = []
let fails = 0
const check = (name, got, want) => {
  const ok = got === want
  if (!ok) fails += 1
  lines.push(`${name}=${got} want=${want} ${ok ? 'PASS' : 'FAIL'}`)
}

const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: 1100, height: 760 } })
await page.addInitScript(({ invite, plant }) => {
  const calls = []
  let keyChanged = !plant
  window.__bbCalls = calls
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    transformCallback: () => 0,
    unregisterCallback: () => {},
    convertFileSrc: (p) => p,
    invoke: async (cmd, a) => {
      calls.push({ cmd, args: a ?? null })
      switch (cmd) {
        case 'sync_status':
          // The window waits for a session revision; a signed-in, unlocked session passes the gate.
          return { logged_in: true, vault_unlocked: true, session_revision: 1 }
        case 'desktop_capabilities':
          // tests/fixtures/desktop-capabilities.json, `windows-nsis`.
          return {"host_os": "windows", "filesystem_integration": "cloud_files", "on_demand": true, "native_credentials": true, "tray": "available", "sharing": "recipient_read_only", "install_format": "nsis", "update_format": "nsis", "sync_mode_selection": false, "metered_sync_control": false, "sync_overlay_control": false, "hydrate_all_control": false, "cache_limit_control": false}
        case 'share_list':
          return { invitations: [], links: [], requests: [], pending: [] }
        case 'share_people':
          return [
            { id: 'me', name: 'Anna Visser', email: 'anna@example.com', you: true, owner: true },
            { id: invite, name: 'Jonas Meier', email: 'jonas@northwind.studio', access: 'Can edit', ...(keyChanged ? { keyChanged: true } : {}) },
          ]
        case 'share_accept_changed_key':
          keyChanged = false
          return null
        case 'share_revoke':
        case 'share_mark_compared':
          return null
        case 'share_safety_number':
          return { groups: Array.from({ length: 12 }, () => '12345') }
        case 'share_invite':
          if (plant || a?.acceptChangedKey) return { status: 'shared' }
          return { status: 'key-changed', memberId: invite }
        default:
          throw new Error(`unknown command ${cmd}`)
      }
    },
  }
}, { invite: INVITE, plant: PLANT })

const calls = (cmd) => page.evaluate((c) => window.__bbCalls.filter((x) => x.cmd === c), cmd)
const title = page.getByText(/’s key has changed/)
const settle = () => page.waitForTimeout(300)

// A step that cannot run (a dialog that never opened) still writes the log: the error is one more failed count.
try {
  await page.goto(`${BASE}/?window=main-app&platform=windows&source=fixtures&section=shared&sub=by-me`)
  // The first by-me row with people (fixtures; since M2 #266 the board's set): its People control is labelled by the
  // people it lists.
  await page.getByRole('button', { name: 'Jonas Meier, Mia de Vries' }).click()
  await settle()

  // 1. People: a share sealed to another key than the person's mark → DW-0515 over People.
  check('people: "’s key has changed"', await title.count(), 1)
  check('people: "Compare safety numbers"', await page.getByRole('button', { name: 'Compare safety numbers' }).count(), 1)
  check('people: "Share anyway"', await page.getByRole('button', { name: 'Share anyway' }).count(), 1)
  await page.screenshot({ path: join(OUT, PLANT ? 'DW-0515-people-planted.png' : 'DW-0515-people.png') })
  if (!PLANT) await page.getByRole('button', { name: 'Share anyway' }).click()
  await settle()
  const acceptedKey = await calls('share_accept_changed_key')
  check('people: share_accept_changed_key calls', acceptedKey.length, 1)
  check('people: accepted the member', acceptedKey[0]?.args?.memberId ?? null, INVITE)
  check('people: alert gone after Share anyway', await title.count(), 0)

  // 2. Invite: the seal is refused (`key-changed`) → DW-0515. Every choice names the refused invite.
  const inviteJonas = async () => {
    await page.getByRole('button', { name: 'Invite someone…' }).click()
    await page.getByLabel('Email').fill('jonas@northwind.studio')
    await page.getByRole('button', { name: 'Send invite' }).click()
    await settle()
  }
  const accepted = async () => (await calls('share_invite')).filter((c) => c.args?.acceptChangedKey === true)
  await inviteJonas()
  check('invite: "’s key has changed"', await title.count(), 1)
  await page.screenshot({ path: join(OUT, PLANT ? 'DW-0515-invite-planted.png' : 'DW-0515-invite.png') })
  const first = await calls('share_invite')
  check('invite: share_invite calls before the choice', first.length, 1)
  check('invite: first call accepts no changed key', first[0]?.args?.acceptChangedKey ?? null, false)

  // 2a. Cancel: shares nothing, and the refused invite (it holds no key) is withdrawn.
  if (!PLANT) await page.getByRole('button', { name: 'Cancel' }).click()
  await settle()
  const revoked = await calls('share_revoke')
  check('cancel: share_revoke calls', revoked.length, 1)
  check('cancel: withdrew the refused invite', revoked[0]?.args?.memberId ?? null, INVITE)
  check('cancel: nothing accepted', (await accepted()).length, 0)
  check('cancel: alert gone', await title.count(), 0)

  // 2b. Share anyway: seals the named refused invite.
  await inviteJonas()
  if (!PLANT) await page.getByRole('button', { name: 'Share anyway' }).click()
  await settle()
  const anyway = await accepted()
  check('share anyway: calls with acceptChangedKey', anyway.length, 1)
  check('share anyway: names the refused invite', anyway[0]?.args?.memberId ?? null, INVITE)
  check('share anyway: alert gone', await title.count(), 0)

  // 2c. Compare safety numbers, They match: the mark, then the seal of that invite.
  await inviteJonas()
  if (!PLANT) await page.getByRole('button', { name: 'Compare safety numbers' }).click()
  await settle()
  check('compare: 5-digit groups', await page.getByText(/^\d{5}$/).count(), 12)
  await page.screenshot({ path: join(OUT, PLANT ? 'DW-0513-refused-planted.png' : 'DW-0513-refused.png') })
  if (!PLANT) await page.getByRole('button', { name: 'They match' }).click()
  await settle()
  const marked = await calls('share_mark_compared')
  check('compare: share_mark_compared calls', marked.length, 1)
  check('compare: They match', marked[0]?.args?.matched ?? null, true)
  const sealed = await accepted()
  check('compare: then sealed the refused invite', sealed.length, 2)
  check('compare: names the refused invite', sealed[1]?.args?.memberId ?? null, INVITE)

  // 2d-2f (desktop #265 delta 1): a refused invite holds no key, so every way out of its comparison but They match
  // withdraws it, as Cancel does: closing the comparison, closing the mismatch, and Remove (it never had access).
  const revokesFrom = async (n) => (await calls('share_revoke')).slice(n)
  const compareRefused = async () => {
    await inviteJonas()
    if (!PLANT) await page.getByRole('button', { name: 'Compare safety numbers' }).click()
    await settle()
  }
  const noMatch = async () => {
    if (!PLANT) await page.getByRole('button', { name: 'They don’t match' }).click()
    await settle()
  }

  // 2d. Compare, then close the comparison (Escape).
  let before = (await calls('share_revoke')).length
  await compareRefused()
  if (!PLANT) await page.keyboard.press('Escape')
  await settle()
  let withdrawn = await revokesFrom(before)
  check('compare, close: share_revoke calls', withdrawn.length, 1)
  check('compare, close: withdrew the refused invite', withdrawn[0]?.args?.memberId ?? null, INVITE)

  // 2e. Compare, They don't match, then close the mismatch dialog.
  before = (await calls('share_revoke')).length
  const marksBefore = (await calls('share_mark_compared')).length
  await compareRefused()
  await noMatch()
  const noMatchMarks = (await calls('share_mark_compared')).slice(marksBefore)
  check('no match: share_mark_compared calls', noMatchMarks.length, 1)
  check('no match: matched', noMatchMarks[0]?.args?.matched ?? null, false)
  check('no match: "Sharing with Jonas is stopped"', await page.getByText(/Sharing with Jonas is stopped/).count(), 1)
  await page.screenshot({ path: join(OUT, PLANT ? 'DW-0513b-refused-planted.png' : 'DW-0513b-refused.png') })
  if (!PLANT) await page.keyboard.press('Escape')
  await settle()
  withdrawn = await revokesFrom(before)
  check('no match, close: share_revoke calls', withdrawn.length, 1)
  check('no match, close: withdrew the refused invite', withdrawn[0]?.args?.memberId ?? null, INVITE)

  // 2f. Compare, They don't match, Remove Jonas's access: withdrawn at once, no "Remove access?" step.
  before = (await calls('share_revoke')).length
  await compareRefused()
  await noMatch()
  if (!PLANT) await page.getByRole('button', { name: 'Remove Jonas’s access' }).click()
  await settle()
  withdrawn = await revokesFrom(before)
  check('no match, remove: share_revoke calls', withdrawn.length, 1)
  check('no match, remove: withdrew the refused invite', withdrawn[0]?.args?.memberId ?? null, INVITE)
  check('no match, remove: no "Remove access?" alert', await page.getByText(/Remove Jonas’s access\?/).count(), 0)
  check('no match, remove: back on People', await page.getByRole('button', { name: 'Invite someone…' }).count(), 1)

} catch (e) {
  fails += 1
  lines.push(`error=${String(e?.message ?? e).split('\n')[0]} FAIL`)
}

lines.push(`fails=${fails}`)
writeFileSync(join(OUT, PLANT ? 'shots-planted.log' : 'shots.log'), `${lines.join('\n')}\n`)
console.log(lines.join('\n'))
await browser.close()
process.exit(fails === 0 ? 0 : 1)
