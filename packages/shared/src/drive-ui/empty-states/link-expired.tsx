// The empty state for an expired or revoked share link.
import { Icon } from '../../design'
import { BBButton } from '../../design'
import { BBLogo } from '../../design'

const honeycombBg = `url("data:image/svg+xml,%3Csvg width='28' height='49' viewBox='0 0 28 49' xmlns='http://www.w3.org/2000/svg'%3E%3Cg fill-rule='evenodd'%3E%3Cg fill='%23f5b800' fill-opacity='1'%3E%3Cpath d='M13.99 9.25l13 7.5v15l-13 7.5L1 31.75v-15l12.99-7.5zM3 17.9v12.7l10.99 6.34 11-6.35V17.9l-11-6.34L3 17.9z'/%3E%3C/g%3E%3C/g%3E%3C/svg%3E")`

interface LinkExpiredProps {
  senderName?: string
  senderInitials?: string
  fileName?: string
  expiredAgo?: string
  expiryDuration?: string
  maxOpens?: number
  onRequestNew?: () => void
}

export function LinkExpired({
  senderName,
  senderInitials,
  fileName,
  expiredAgo = '2h ago',
  expiryDuration = '24 hours',
  maxOpens = 3,
  onRequestNew,
}: LinkExpiredProps) {
  return (
    <div className="min-h-screen flex items-center justify-center bg-paper-2 p-6">
      <div
        className="w-full max-w-[560px] min-h-[480px] p-9 rounded-xl border border-line flex flex-col"
        style={{ background: 'var(--color-paper-2)' }}
      >
        <BBLogo size={14} />

        <div className="flex-1 flex items-center justify-center py-7">
          <div className="max-w-[420px] text-center">
            {/* Clock icon */}
            <div className="w-[52px] h-[52px] mx-auto mb-4 rounded-2xl flex items-center justify-center bg-paper border border-line text-ink-3">
              <Icon name="clock" size={22} />
            </div>

            <h2 className="text-lg font-semibold text-ink mb-2.5">
              This link has expired
            </h2>
            <p className="text-[13px] text-ink-3 leading-relaxed mb-5">
              Links from Beebeeb self-destruct by design. The sender set
              this one to expire after{' '}
              <span className="font-mono text-ink-2">{expiryDuration}</span>{' '}
              or{' '}
              <span className="font-mono text-ink-2">{maxOpens} opens</span>,
              whichever came first.
            </p>

            {/* Sender card */}
            {senderName && (
              <div className="flex items-center gap-3 p-3 mb-4 bg-paper border border-line rounded-md text-left">
                <div className="w-8 h-8 rounded-full bg-amber text-ink font-bold text-xs flex items-center justify-center shrink-0">
                  {senderInitials ?? senderName.slice(0, 2).toUpperCase()}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="text-[12.5px] font-medium">{senderName}</div>
                  <div className="font-mono text-[10.5px] text-ink-3">
                    shared{' '}
                    {fileName && (
                      <span className="text-ink-2">{fileName}</span>
                    )}
                    {fileName && ' · '}expired {expiredAgo}
                  </div>
                </div>
              </div>
            )}

            {onRequestNew && (
              <BBButton
                variant="amber"
                size="lg"
                className="w-full justify-center gap-1.5"
                onClick={onRequestNew}
              >
                <Icon name="share" size={13} /> Request a new link
              </BBButton>
            )}

            <p className="text-[11px] text-ink-4 mt-3">
              Beebeeb never saw the file&apos;s contents. We can&apos;t
              re-open expired links for you.
            </p>
          </div>
        </div>

        {/* Honeycomb watermark */}
        <div
          className="absolute inset-0 opacity-[0.04] pointer-events-none rounded-xl overflow-hidden"
          style={{ backgroundImage: honeycombBg }}
        />
      </div>
    </div>
  )
}
