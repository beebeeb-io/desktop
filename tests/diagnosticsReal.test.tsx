// Diagnostics names every read: the command that answered, and the time it answered.
// One failed read stays on the page, named, with Try again. The other rows stay.
// A saved bundle shows the path report_problem returned, never a folder this file guessed.
import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { commandUnavailableLabel, type AppActivitySnapshot } from '../src/desktopApi'
import type { ProblemReportResult } from '../src/diagnosticsCopy'
import { Ctx, type SettingsCtx } from '../src/v2/sections/settings/shell/context'
import { Diagnostics, formatReadAt } from '../src/v2/sections/settings/today/diagnostics'

const ctx: SettingsCtx = { pane: 'diagnostics', go() {}, dialog: null, open() {}, close() {} }

const textOf = (html: string) => html
  .replace(/<[^>]+>/g, ' ')
  .replace(/&#x27;|&#39;/g, "'")
  .replace(/&amp;/g, '&')
  .replace(/&quot;/g, '"')
  .replace(/\s+/g, ' ')
  .trim()

const ACTIVITY_AT = '2026-11-09T08:12:00.000Z'
const ACTIVITY_CLOCK = formatReadAt(ACTIVITY_AT)
const REPORT_AT = '2026-11-09T09:13:00.000Z'
const REPORT_CLOCK = formatReadAt(REPORT_AT)

const snapshot: AppActivitySnapshot = {
  pid: 4242,
  process_name: 'beebeeb',
  cpu_percent: 3.5,
  memory_rss_bytes: 52_428_800,
  upload_bps: 2048,
  download_bps: null,
}

const answered: ProblemReportResult = {
  path: 'D:/Bundles/answered-by-report-problem.json',
  email_opened: true,
}

function draw(props: {
  saved?: boolean
  failed?: boolean
  report?: ProblemReportResult
  reportReadAt?: string | null
  activity?: { ok: true; value: AppActivitySnapshot; readAt: string } | { ok: false; unsupported: boolean; readAt: string | null; reason?: string }
} = {}) {
  const html = renderToStaticMarkup(createElement(Ctx.Provider, { value: ctx }, createElement(Diagnostics, props)))
  return { html, text: textOf(html) }
}

describe('Diagnostics shows what it read', () => {
  test('a failed activity read names that row and the other rows stay', () => {
    const view = draw({
      activity: { ok: false, unsupported: false, readAt: ACTIVITY_AT },
      saved: true,
      report: answered,
      reportReadAt: REPORT_AT,
    })
    expect(view.text).toContain('App activity')
    expect(view.text).toContain('app_activity_snapshot')
    expect(view.text).toContain(ACTIVITY_CLOCK)
    expect(view.text).toContain('Beebeeb couldn’t read its activity on this PC.')
    expect(view.html.match(/Try again/g)).toHaveLength(1)
    expect(view.text).not.toContain('0.0 %')
    expect(view.text).not.toContain('3.5 %')
    expect(view.text).toContain(answered.path)
    expect(view.text).toContain('report_problem')
    expect(view.text).toContain(REPORT_CLOCK)
    expect(view.text).toContain('Queue counts, an error code and the last error message.')
    expect(view.text).toContain('Paths, file and folder names, sign-in tokens.')
    expect(view.text).toContain('Kept: standard error words, numbers, opaque ids and the server address.')
    expect(view.text).toContain('The email is sent from your own address, so support sees it.')
    expect(view.text).toContain('Test connection speed')
    expect(view.text).toContain('Get help')
    expect(view.text).toContain('Report a problem')
    expect(view.text).not.toContain('%LOCALAPPDATA%')
    expect(view.text).not.toContain('beebeeb-diagnostics-1793956320')

    const refused = draw({
      activity: { ok: false, unsupported: true, readAt: ACTIVITY_AT, reason: 'app_activity_snapshot: not implemented in a browser' },
      saved: true,
      report: answered,
      reportReadAt: REPORT_AT,
    })
    expect(refused.text).toContain('App activity')
    expect(refused.text).toContain('app_activity_snapshot')
    expect(refused.text).toContain(commandUnavailableLabel('app_activity_snapshot'))
    expect(refused.text).not.toContain('not implemented')
    expect(refused.text).toContain(answered.path)
    expect(refused.html.match(/Try again/g)).toHaveLength(1)
  })

  test('a saved report shows the path the command returned', () => {
    const view = draw({
      activity: { ok: true, value: snapshot, readAt: ACTIVITY_AT },
      saved: true,
      report: answered,
      reportReadAt: REPORT_AT,
    })
    expect(view.text).toContain('Support bundle saved')
    expect(view.text).toContain(answered.path)
    expect(view.text).toContain('report_problem')
    expect(view.text).toContain(REPORT_CLOCK)
    expect(view.text).toContain('A draft to support is opening in your email app; attach the file yourself.')
    expect(view.text).not.toContain('%LOCALAPPDATA%')
    expect(view.text).not.toContain('beebeeb-diagnostics-1793956320')

    const closed = draw({
      activity: { ok: true, value: snapshot, readAt: ACTIVITY_AT },
      saved: true,
      report: { path: answered.path, email_opened: false },
      reportReadAt: REPORT_AT,
    })
    expect(closed.text).toContain(answered.path)
    expect(closed.text).toContain('report_problem')
    expect(closed.text).toContain("Your email app didn't open; send the file to support@beebeeb.io yourself.")

    const missing = draw({ saved: true, activity: { ok: true, value: snapshot, readAt: ACTIVITY_AT } })
    expect(missing.text).not.toContain('answered-by-report-problem')
    expect(missing.text).not.toContain('%LOCALAPPDATA%')
    expect(missing.text).not.toContain('Support bundle saved')
  })

  test('each activity row names app_activity_snapshot and when it was read', () => {
    const view = draw({
      activity: { ok: true, value: snapshot, readAt: ACTIVITY_AT },
      failed: true,
    })
    for (const name of ['CPU', 'Memory', 'Upload', 'Download']) expect(view.text).toContain(name)
    expect(view.text.match(/app_activity_snapshot/g)).toHaveLength(4)
    expect(view.text.match(new RegExp(ACTIVITY_CLOCK.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g'))).toHaveLength(4)
    expect(view.text).toContain('3.5 %')
    expect(view.text).toContain('50 MB')
    expect(view.text).toContain('2 KB/s')
    expect(view.text).not.toContain('0 KB/s')
    expect(view.text).toContain("Couldn't save the support bundle")
    expect(view.text).toContain('report_problem')
    expect(view.text).toContain('The folder for support bundles could not be written to.')
    expect(view.html.match(/Try again/g)).toHaveLength(1)
    expect(view.text).not.toMatch(/\b(EACCES|mutex poisoned|not implemented)\b/)
  })
})
