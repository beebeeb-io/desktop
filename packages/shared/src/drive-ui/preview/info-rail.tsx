/**
 * The content cipher every Beebeeb chunk is sealed with (core `encrypt_chunk`,
 * cipher_suite `V1Aes256Gcm`). Callers pass it explicitly so the rail never
 * renders a cipher that isn't the file's.
 */
export const CONTENT_CIPHER_LABEL = 'AES-256-GCM'

interface InfoRailProps {
  filename: string
  kind: string
  size: string
  items?: [string, string][]
  /** Human label of the cipher this file's content is encrypted with. */
  cipher: string
  /** Number of encrypted chunks the file is stored as, if known. */
  chunkCount?: number
  /**
   * True only after THIS file's full ciphertext was decrypted in this browser —
   * every chunk's GCM authentication tag was checked (a bad tag makes the
   * decrypt throw). False while decrypting, on error, or when the preview is
   * showing an encrypted thumbnail instead of the original.
   */
  tagsVerified?: boolean
  /**
   * LD-0001 PR-B: the streaming line (`Streaming · 41 of 120 parts verified`
   * or `All 120 parts verified`). It reports only parts OPENED in this
   * session — tags, never order or completeness.
   */
  streamLine?: string
}

export function InfoRail({
  filename,
  kind,
  size,
  items = [],
  cipher,
  chunkCount,
  tagsVerified = false,
  streamLine,
}: InfoRailProps) {
  const rows: [string, string][] = [
    ['Name', filename],
    ['Size', size],
    ['Kind', kind],
    ...items,
  ]

  return (
    <div className="flex flex-col gap-[18px]">
      {/* Details section */}
      <div>
        <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-ink-3">
          Details
        </div>
        {rows.map(([label, value], i) => (
          <div
            key={i}
            className="flex border-b border-line py-[5px] text-xs"
          >
            <span className="w-[90px] shrink-0 text-ink-3">{label}</span>
            <span className="flex-1 break-all font-mono text-[11px] text-ink">
              {value}
            </span>
          </div>
        ))}
      </div>

      {/* Crypto section */}
      <div>
        <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-ink-3">
          Crypto
        </div>
        <div
          data-testid="preview-crypto"
          className="font-mono text-[11px] leading-[1.8] text-ink-2"
        >
          <div>{cipher}</div>
          <div>256-bit per-file key</div>
          {streamLine && <div data-testid="preview-stream-line">{streamLine}</div>}
          {chunkCount !== undefined && chunkCount > 0 && (
            <div>
              {chunkCount} {chunkCount === 1 ? 'chunk' : 'chunks'}
            </div>
          )}
          {tagsVerified && <div>GCM tags verified on decrypt &#10003;</div>}
        </div>
      </div>
    </div>
  )
}
