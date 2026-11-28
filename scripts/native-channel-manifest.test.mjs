import assert from 'node:assert/strict'
import { test } from 'node:test'
import { keptEntries } from './native-channel-manifest.mjs'

const win = { url: 'w', signature: 's', size: 1 }
const mac = { url: 'm', signature: 's', size: 1 }
const existing = (version, platforms) => ({ product: 'desktop', channel: 'stable', version, platforms })
const id = { version: '0.9.1', channel: 'stable' }

test('keeps the other OS for the same version', () => {
  const e = existing('0.9.1', { 'windows-x86_64': win, 'darwin-aarch64': mac })
  assert.deepEqual(Object.keys(keptEntries(e, id, ['darwin-aarch64'])), ['windows-x86_64'])
  assert.deepEqual(Object.keys(keptEntries(e, id, ['windows-x86_64'], { sameVersionOnly: true })), ['darwin-aarch64'])
})
test('Windows publish of a newer version starts fresh', () => {
  assert.deepEqual(keptEntries(existing('0.9.0', { 'darwin-aarch64': mac }), id, ['windows-x86_64']), {})
})
test('Mac publish refuses another version or a lower one', () => {
  const opts = { sameVersionOnly: true }
  assert.throws(() => keptEntries(existing('0.9.0', { 'windows-x86_64': win }), id, ['darwin-aarch64'], opts), /not 0\.9\.1/)
  assert.throws(() => keptEntries(existing('0.9.2', { 'windows-x86_64': win }), id, ['darwin-aarch64'], opts), /lower 0\.9\.1/)
})
test('no publish may lower the channel version', () => {
  assert.throws(() => keptEntries(existing('0.9.2', {}), id, ['windows-x86_64']), /lower/)
})
test('a withdrawal (allowLower, publish_existing) may publish a lower version; without it the same call throws', () => {
  const higher = existing('0.9.2', { 'windows-x86_64': win })
  assert.throws(() => keptEntries(higher, id, ['windows-x86_64']), /lower/)
  assert.deepEqual(keptEntries(higher, id, ['windows-x86_64'], { allowLower: true }), {})
  // A Mac-only publish for another version is still refused: it would strand the Windows entries.
  assert.throws(() => keptEntries(higher, id, ['darwin-aarch64'], { sameVersionOnly: true, allowLower: true }), /not 0\.9\.1/)
})
test('no existing manifest keeps nothing; another channel throws', () => {
  assert.deepEqual(keptEntries(undefined, id, ['darwin-aarch64'], { sameVersionOnly: true }), {})
  assert.throws(() => keptEntries({ ...existing('0.9.1', {}), channel: 'beta' }, id, []), /beta/)
})
