// Walkthrough for slice 66 (the Files toolbar is a search field). Run against `bunx vite --port 5251`.
// a) DW-0801 and DW-1105 reach the search page (its own field under the Search section): 2 of 2 frames.
// b) the plain Files window has one labelled field, Ctrl+F, no Filter, and keeps Sort / as List / as Grid.
// c) clicking the field and pressing Enter opens the Search section once.
import { chromium } from 'playwright'

const BASE = 'http://localhost:5251'
const stateRoute = (state) => `${BASE}/?window=main-app&platform=windows&state=${state}`
const plainRoute = `${BASE}/?window=main-app&platform=windows`

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || '/usr/bin/chromium' })
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
const errors = []
page.on('pageerror', (error) => errors.push(String(error)))

async function ready(state) {
  await page.goto(stateRoute(state), { waitUntil: 'domcontentloaded' })
  await page.waitForFunction(() => document.documentElement.dataset.v2StateResult === 'ready', null, { timeout: 20000 })
}

// a) the two search frames
let frames = 0
for (const state of ['DW-0801', 'DW-1105']) {
  await ready(state)
  const section = page.locator('section[aria-label="Search"]')
  await section.waitFor({ state: 'visible', timeout: 10000 })
  const field = section.locator('input[aria-label="Search names"]')
  await field.waitFor({ state: 'visible', timeout: 10000 })
  const dataSearch = await section.getAttribute('data-search')
  console.log(`frame ${state}: section=1 field=1 data-search=${dataSearch}`)
  frames += 1
}

// b) the plain Files window
await page.goto(plainRoute, { waitUntil: 'domcontentloaded' })
const toolbar = page.locator('div.h-12').first()
const fields = toolbar.locator('input[aria-label="Search names"][placeholder="Search names"]')
await fields.first().waitFor({ state: 'visible', timeout: 20000 })
const fieldCount = await fields.count()
const toolbarText = (await toolbar.innerText()).replace(/\s+/g, ' ').trim()
const ctrlF = toolbarText.includes('Ctrl+F')
const filterCount = await page.getByRole('button', { name: 'Filter', exact: true }).count()
const sortCount = await page.getByRole('button', { name: 'Sort', exact: true }).count()
const listCount = await page.getByRole('button', { name: 'as List', exact: true }).count()
const gridCount = await page.getByRole('button', { name: 'as Grid', exact: true }).count()
console.log(`files: fields=${fieldCount} ctrlF=${ctrlF} filter=${filterCount} sort=${sortCount} asList=${listCount} asGrid=${gridCount}`)

// c) click the field, press Enter: the Search section opens once
const before = await page.locator('section[aria-label="Search"]').count()
await fields.first().click()
const focused = await page.evaluate(() => document.activeElement?.getAttribute('aria-label'))
console.log(`enter: focused=${focused}`)
await page.keyboard.press('Enter')
await page.waitForSelector('section[aria-label="Search"]', { state: 'attached', timeout: 10000 })
const after = await page.locator('section[aria-label="Search"]').count()
console.log(`enter: url=${page.url()} searchSectionsBefore=${before} searchSectionsAfter=${after}`)

console.log(`frames=${frames}/2 pageErrors=${errors.length}`)
if (errors.length) console.log(errors.join('\n'))
await browser.close()

const ok = frames === 2 && errors.length === 0 && fieldCount === 1 && ctrlF
  && filterCount === 0 && sortCount === 1 && listCount === 1 && gridCount === 1
  && before === 0 && after === 1
process.exit(ok ? 0 : 1)
