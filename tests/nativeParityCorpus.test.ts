import { expect, test } from 'bun:test'
import { readFileSync, existsSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { resolve } from 'node:path'

const root = resolve(import.meta.dir, '../fixtures/native-parity')
test('native parity corpus has nine scenarios, two accounts and fourteen stable objects', () => {
  expect(existsSync(`${root}/manifest.json`)).toBe(true)
  const corpus = JSON.parse(readFileSync(`${root}/manifest.json`, 'utf8'))
  expect(corpus.accounts).toEqual(['alice', 'bob'])
  expect(corpus.scenarios.map((s: { id: string }) => s.id).sort()).toEqual([
    'edit-conflict', 'expired-session', 'image-thumbnail', 'interrupted-upload',
    'nested-tree', 'revoked-share', 'subtree-trash', 'two-accounts', 'video-poster',
  ])
  expect(corpus.objects).toHaveLength(14)
  expect(new Set(corpus.objects.map((o: { id: string }) => o.id)).size).toBe(14)
  let hashes = 0
  for (const object of corpus.objects) {
    for (const version of [...object.versions, ...object.thumbnails]) {
      const bytes = readFileSync(`${root}/${version.source}`)
      expect(bytes.length).toBe(version.size)
      expect(createHash('sha256').update(bytes).digest('hex')).toBe(version.sha256)
      hashes++
    }
  }
  expect(hashes).toBe(16)
})
