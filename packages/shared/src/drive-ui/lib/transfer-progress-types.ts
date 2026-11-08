// The progress a whole-file preview load reports (the web download pipeline and desktop's engine both produce it).
export interface TransferProgress {
  phase: 'downloading' | 'decrypting' | 'done'
  receivedBytes: number
  totalBytes: number | null
  decryptedChunks: number
  totalChunks: number
}
