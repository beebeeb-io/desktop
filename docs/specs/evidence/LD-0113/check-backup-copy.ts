// Checks the person-facing strings of section 5's four windows against the desktop's own guard lists.
//   bun docs/specs/evidence/LD-0113/check-backup-copy.ts design/hifi/macos-settings-dialogs.html
// Proven red on a mutant (a developer word, a city, the offline promise, a straight apostrophe): 5 offenders, exit 1.
import { readFileSync } from 'node:fs'
import { DEVELOPER_WORDS } from '../../../../tests/fixtures/copyGuard'
import { CITY_NAME } from '../../../../tests/fixtures/cityGuard'

const html = readFileSync(process.argv[2], 'utf8')
const start = html.indexOf('<section class="bk"')
const end = html.indexOf('</section>\n', html.lastIndexOf('<p class="sub"><strong>Not drawn here:</strong>'))
if (start < 0 || end < 0) throw new Error('section 5 not found')
const section = html.slice(start, end)

// Person-facing strings: the text nodes and aria-labels inside the four .mw-window drawings.
const windows = [...section.matchAll(/<div class="mw-window" id="(bk-[a-z]+)"[\s\S]*?<\/main><\/div>/g)]
const strings = new Set<string>()
for (const w of windows) {
  for (const m of w[0].matchAll(/aria-label="([^"]+)"/g)) strings.add(m[1])
  for (const m of w[0].replace(/<svg[\s\S]*?<\/svg>/g, '').matchAll(/>([^<>]+)</g)) if (m[1].trim()) strings.add(m[1].trim())
}
const FORBIDDEN: Array<[string, RegExp]> = [
  ...DEVELOPER_WORDS.map((w) => [w.id, w.pattern] as [string, RegExp]),
  ['city', CITY_NAME],
  ['offline promise', /\boffline\b|kept on this|available offline|always available/i],
  ['OneDrive on a Mac', /onedrive/i],
  ['emoji', /\p{Extended_Pictographic}/u],
  ['straight apostrophe', /'/],
]
const offenders: string[] = []
for (const s of strings) for (const [id, re] of FORBIDDEN) if (re.test(s)) offenders.push(`${id}: ${s}`)
// The section's own prose: no city, no emoji, anywhere.
const prose = section.replace(/<[^>]+>/g, ' ')
for (const [id, re] of [['city', CITY_NAME], ['emoji', /\p{Extended_Pictographic}/u]] as Array<[string, RegExp]>) if (re.test(prose)) offenders.push(`${id} in prose`)
console.log(`${windows.length} windows, ${strings.size} distinct person-facing strings, ${FORBIDDEN.length} rules, ${offenders.length} offenders`)
for (const o of offenders) console.log('  ' + o)
process.exit(offenders.length || windows.length !== 4 ? 1 : 0)
