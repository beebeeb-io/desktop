/**
 * LD-0001 §7 — the exact progress copy for a whole-file preview load.
 *
 * Pure: no React, no fetch. The caller owns the bar; this owns the words.
 */
import type { TransferProgress } from './transfer-progress-types'

function unitFor(bytes: number): { div: number; label: string; decimals: number } {
  if (bytes < 1024) return { div: 1, label: 'B', decimals: 0 }
  if (bytes < 1024 ** 2) return { div: 1024, label: 'KB', decimals: 1 }
  if (bytes < 1024 ** 3) return { div: 1024 ** 2, label: 'MB', decimals: 1 }
  return { div: 1024 ** 3, label: 'GB', decimals: 1 }
}

export function progressCopy(p: TransferProgress): string {
  if (p.phase === 'decrypting') {
    const part = Math.min(p.decryptedChunks + 1, p.totalChunks)
    return `Decrypting part ${part} of ${p.totalChunks}`
  }
  if (p.phase === 'done') return 'Download complete'

  if (p.totalBytes === null) {
    // No total yet: the received count still names its own unit in the same
    // MB-style wording ("Downloading 12.4 MB").
    const own = unitFor(p.receivedBytes)
    return `Downloading ${(p.receivedBytes / own.div).toFixed(own.decimals)} ${own.label}`
  }
  // One unit for both numbers, chosen by the total: "12.4 of 48.0 MB".
  const u = unitFor(p.totalBytes)
  const received = (p.receivedBytes / u.div).toFixed(u.decimals)
  const total = (p.totalBytes / u.div).toFixed(u.decimals)
  const base = `Downloading ${received} of ${total} ${u.label}`
  if (p.decryptedChunks === 0) return base
  return `${base} · ${p.decryptedChunks} of ${p.totalChunks} parts decrypted`
}
