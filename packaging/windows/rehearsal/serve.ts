// Serves one folder on 127.0.0.1 for the upgrade rehearsal (packaging/windows/README.md): the local update
// manifest and the installer it points at. Loopback only; plain files; honours Range like the real host.
//   bun packaging/windows/rehearsal/serve.ts <folder> [port]
import { join, normalize, sep } from 'node:path'

const root = normalize(process.argv[2] ?? '')
const port = Number(process.argv[3] ?? 8787)
if (!process.argv[2]) throw new Error('usage: bun serve.ts <folder> [port]')

Bun.serve({
  hostname: '127.0.0.1',
  port,
  async fetch(request) {
    const path = normalize(join(root, decodeURIComponent(new URL(request.url).pathname)))
    if (path !== root && !path.startsWith(root + sep)) return new Response('no', { status: 403 })
    const file = Bun.file(path)
    if (!(await file.exists())) {
      console.log(`404 ${request.url}`)
      return new Response('not found', { status: 404 })
    }
    const range = /^bytes=(\d+)-$/.exec(request.headers.get('range') ?? '')
    console.log(`${range ? 206 : 200} ${request.url}${range ? ` from ${range[1]}` : ''}`)
    if (!range) return new Response(file)
    const from = Number(range[1])
    return new Response(file.slice(from), { status: 206, headers: { 'Content-Range': `bytes ${from}-${file.size - 1}/${file.size}` } })
  },
})
console.log(`serving ${root} on http://127.0.0.1:${port}`)
