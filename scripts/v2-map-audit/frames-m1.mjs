// DW-A84a and DW-A57a (M1): one screenshot each from the dev server on port 5221, each asserting counts. Usage: bun scripts/v2-map-audit/frames-m1.mjs [out-dir]
import { chromium } from 'playwright'
const out = process.argv[2] ?? `${process.env.HOME}/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/m1`
const b = await chromium.launch(); const p = await b.newPage({ viewport: { width: 1280, height: 800 } })
let fail = 0
const check = (id, what, got, want) => { const ok = got === want; if (!ok) fail++; console.log(`${id}: ${what} = ${got} (want ${want}) ${ok ? 'PASS' : 'FAIL'}`) }
await p.goto('http://localhost:5221/?window=main-app&platform=windows&frame=DW-A84a'); await p.waitForTimeout(1500)
check('DW-A84a', "'6 of 7 done' headings/text", await p.getByText('6 of 7 done', { exact: true }).count(), 1)
check('DW-A84a', "to-do 'Add' buttons", await p.getByRole('button', { name: 'Add', exact: true }).count(), 1)
const done = ['Email verified', 'Two-factor authentication', 'Strong password', 'Recovery phrase saved', 'Password not in a breach', 'Devices and sessions']
let n = 0; for (const t of done) n += await p.locator('main, body').getByRole('button', { name: new RegExp('^' + t) }).count() > 0 ? 1 : 0
check('DW-A84a', 'done rows present', n, 6)
await p.screenshot({ path: `${out}/DW-A84a.png` })
await p.goto('http://localhost:5221/?window=main-app&platform=windows&frame=DW-A57a'); await p.waitForTimeout(1500)
check('DW-A57a', 'dialogs', await p.getByRole('dialog').or(p.getByRole('alertdialog')).count(), 1)
check('DW-A57a', "'Sign out of all Beebeeb apps on this PC?' heading", await p.getByRole('heading', { name: 'Sign out of all Beebeeb apps on this PC?' }).count(), 1)
check('DW-A57a', "'Sign out of all' buttons", await p.getByRole('button', { name: 'Sign out of all', exact: true }).count(), 1)
await p.screenshot({ path: `${out}/DW-A57a.png` })
await b.close(); process.exit(fail ? 1 : 0)
