// P1b base item 6: prove the dev server actually serves the isolation headers on the page and on module URLs, not
// just in the config. Starts `bunx vite --port <port> --strictPort`, waits for the port, fetches `/`, then the page's
// own script URL and one module it imports. Every fetch must answer 200 with COOP same-origin + COEP require-corp.
// The vite child is stopped by the PID recorded at spawn (never by name). Exit 0 = both headers on html and 2 assets.
//
//   node scripts/check-v2-dev-headers.mjs --port 5241
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { connect } from 'node:net'
import { fileURLToPath } from 'node:url'

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => {
    if (a.startsWith('--')) acc.push([a.slice(2), all[i + 1] && !all[i + 1].startsWith('--') ? all[i + 1] : 'true'])
    return acc
  }, []),
)
const PORT = Number(args.port ?? 5173)
const ROOT = fileURLToPath(new URL('..', import.meta.url))
const BASE = `http://localhost:${PORT}`

const COOP = 'cross-origin-opener-policy'
const COEP = 'cross-origin-embedder-policy'
const WANT = { [COOP]: 'same-origin', [COEP]: 'require-corp' }

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

function portOpen(port, host = '127.0.0.1', timeout = 300) {
  return new Promise((resolve) => {
    const socket = connect({ port, host })
    const done = (open) => {
      socket.destroy()
      resolve(open)
    }
    socket.setTimeout(timeout)
    socket.once('connect', () => done(true))
    socket.once('timeout', () => done(false))
    socket.once('error', () => done(false))
  })
}

/** Vite's default `localhost` can land on ::1 or 127.0.0.1; either counts as the port being open. */
const anyPortOpen = async (port, timeout = 300) => (await portOpen(port, '127.0.0.1', timeout)) || portOpen(port, '::1', timeout)

async function waitForPort(port, maxMs = 30_000) {
  const until = Date.now() + maxMs
  while (Date.now() < until) {
    if (await anyPortOpen(port)) return true
    await sleep(150)
  }
  return false
}

const headersOf = (res) => ({ [COOP]: res.headers.get(COOP), [COEP]: res.headers.get(COEP) })
const passes = (h) => h[COOP] === WANT[COOP] && h[COEP] === WANT[COEP]

async function fetchWith(path) {
  return fetch(`${BASE}${path}`, { redirect: 'manual', signal: AbortSignal.timeout(15_000) })
}

/** The page's module URL plus one module that URL imports, both same-origin paths. */
function sameOriginModules(html, mainBody) {
  const found = []
  const own = [...html.matchAll(/(?:src|href)="(\/[^"]+)"/g)]
    .map((m) => m[1])
    .find((u) => /\.(?:m?[jt]sx?|css)(?:[?#]|$)/.test(u))
  if (own) found.push(own)
  if (mainBody) {
    for (const m of mainBody.matchAll(/(?:from|import)\s*\(?\s*["'](\/[^"']+)["']/g)) {
      if (!found.includes(m[1])) found.push(m[1])
    }
  }
  return found.slice(0, 2)
}

let vite
let viteLog = ''

async function stopVite() {
  if (!vite || vite.exitCode !== null) return
  const pid = vite.pid
  vite.kill('SIGTERM')
  const stopped = await Promise.race([once(vite, 'exit').then(() => true), sleep(4000).then(() => false)])
  if (!stopped && vite.exitCode === null) vite.kill('SIGKILL')
  if (vite.exitCode === null) await once(vite, 'exit').catch(() => {})
  // The port must be free again; otherwise a grandchild outlived the recorded PID.
  for (let i = 0; i < 20 && (await anyPortOpen(PORT, 200)); i += 1) await sleep(100)
  if (await anyPortOpen(PORT, 200)) {
    console.error(`port ${PORT} still open after stopping vite pid ${pid}`)
    process.exitCode = 1
  }
}

async function main() {
  vite = spawn('bunx', ['vite', '--port', String(PORT), '--strictPort'], {
    cwd: ROOT,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, BROWSER: 'none' },
  })
  const pid = vite.pid
  vite.stdout.on('data', (d) => (viteLog += d))
  vite.stderr.on('data', (d) => (viteLog += d))
  vite.on('exit', (code, signal) => {
    if (code !== 0 && code !== null) viteLog += `\nvite exited code=${code} signal=${signal}\n`
  })

  if (!(await waitForPort(PORT))) {
    console.error(`vite did not open port ${PORT} (pid ${pid})\n${viteLog}`)
    process.exitCode = 1
    return
  }

  const htmlRes = await fetchWith('/')
  const html = await htmlRes.text()
  const pageHeaders = headersOf(htmlRes)

  if (!passes(pageHeaders)) {
    console.log(`coop=${pageHeaders[COOP] ?? 'missing'} coep=${pageHeaders[COEP] ?? 'missing'}`)
    process.exitCode = 1
    return
  }

  const [firstUrl] = sameOriginModules(html)
  if (!firstUrl) {
    console.error(`no module URL in the dev server's html\n${viteLog}`)
    process.exitCode = 1
    return
  }
  const firstRes = await fetchWith(firstUrl)
  const firstBody = await firstRes.text()
  const first = { url: firstUrl, status: firstRes.status, headers: headersOf(firstRes) }

  // The second module is one the first imports; `/@vite/client` is vite's own module when it names none.
  const secondUrl = sameOriginModules(html, firstBody).find((u) => u !== firstUrl) ?? '/@vite/client'
  const secondRes = await fetchWith(secondUrl)
  const second = { url: secondUrl, status: secondRes.status, headers: headersOf(secondRes) }

  const modules = [first, second]
  const ok = modules.filter((r) => r.status === 200 && passes(r.headers)).length
  console.log(`coop=${pageHeaders[COOP]} coep=${pageHeaders[COEP]} html=${htmlRes.status} assets=${ok}`)
  if (htmlRes.status !== 200 || ok !== 2) {
    console.error(`module URLs checked: ${modules.map((m) => `${m.url} -> ${m.status}`).join(', ')}\n${viteLog}`)
    process.exitCode = 1
  }
}

try {
  await main()
} catch (err) {
  console.error(`header check failed: ${err instanceof Error ? err.message : String(err)}\n${viteLog}`)
  process.exitCode = 1
} finally {
  await stopVite()
}
