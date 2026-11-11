// Prints a fingerprint of what a state.db HOLDS (not of its bytes: SQLite may checkpoint its WAL without changing
// a row). The upgrade rehearsal runs it before and after the update and compares the two outputs.
//   bun packaging/windows/rehearsal/db-fingerprint.ts "%LOCALAPPDATA%\io.beebeeb.app.test\state.db"
// It never opens the live file: it copies state.db, -wal and -shm to a temp folder and reads the copy. It prints
// table names, row counts and a hash per table: no row content, no file name, no key.
import { Database } from 'bun:sqlite'
import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const source = process.argv[2]
if (!source || !existsSync(source)) throw new Error('usage: bun db-fingerprint.ts <path to state.db>')
const dir = mkdtempSync(join(tmpdir(), 'bb-db-fingerprint-'))
try {
  for (const suffix of ['', '-wal', '-shm']) if (existsSync(source + suffix)) copyFileSync(source + suffix, join(dir, `state.db${suffix}`))
  const db = new Database(join(dir, 'state.db'))
  const integrity = (db.query('PRAGMA integrity_check').get() as { integrity_check: string }).integrity_check
  const tables = db.query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all() as { name: string }[]
  const all = createHash('sha256')
  let rows = 0
  for (const { name } of tables) {
    const hash = createHash('sha256')
    let count = 0
    const quoted = `"${name.replaceAll('"', '""')}"`
    // A WITHOUT ROWID table has no rowid: order by its columns instead.
    let query
    try {
      query = db.query(`SELECT * FROM ${quoted} ORDER BY rowid`)
    } catch {
      query = db.query(`SELECT * FROM ${quoted} ORDER BY 1, 2`)
    }
    for (const row of query.values()) {
      hash.update(JSON.stringify(row.map((value) => (value instanceof Uint8Array ? Buffer.from(value).toString('hex') : value))))
      count += 1
    }
    const digest = hash.digest('hex')
    all.update(`${name}:${count}:${digest}\n`)
    rows += count
    console.log(`${name}\t${count}\t${digest.slice(0, 16)}`)
  }
  db.close()
  console.log(`FINGERPRINT integrity=${integrity} tables=${tables.length} rows=${rows} sha256=${all.digest('hex')}`)
  if (integrity !== 'ok' || tables.length === 0) process.exit(1)
} finally {
  rmSync(dir, { recursive: true, force: true })
}
