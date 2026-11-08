// The empty state shown when the recovery phrase is unavailable.
import type { ReactNode } from 'react'
import { Icon } from '../../design'
import { BBButton } from '../../design'
import { BBLogo } from '../../design'
import { BBChip } from '../../design'
import type { IconName } from '../../design'

interface RecoveryOption {
  icon: IconName
  title: string
  description: string
  cta?: string
  onAction?: () => void
}

interface ForgotPhraseProps {
  /** Recovery paths the user can still try */
  recoveryOptions?: RecoveryOption[]
  onStartOver: () => void
  onFoundPhrase?: () => void
  /** Optional extra content after the dead-end card */
  children?: ReactNode
}

const defaultRecoveryOptions: RecoveryOption[] = [
  {
    icon: 'users',
    title: 'Ask your trusted contact',
    description:
      'If you set up a trusted helper, they can authorize recovery with their own key. Takes a 72h waiting period.',
    cta: 'Send request',
  },
  {
    icon: 'cloud',
    title: 'Check other devices',
    description:
      'If another device is still signed in, open Beebeeb there and re-export the phrase from Settings > Security.',
    cta: 'Show me how',
  },
  {
    icon: 'download',
    title: 'Find the PDF you downloaded',
    description:
      'We nudged you to save a recovery PDF during onboarding. Search your Downloads folder for "beebeeb-recovery.pdf".',
  },
]

export function ForgotPhrase({
  recoveryOptions = defaultRecoveryOptions,
  onStartOver,
  onFoundPhrase,
  children,
}: ForgotPhraseProps) {
  return (
    <div className="min-h-screen flex items-center justify-center bg-paper p-6">
      <div
        className="w-full max-w-[620px] bg-paper border border-line-2 rounded-xl overflow-hidden"
        style={{ boxShadow: 'var(--shadow-2)' }}
      >
        {/* Header */}
        <div className="px-[22px] py-3.5 border-b border-line flex items-center">
          <BBLogo size={14} />
          <BBChip>
            <span
              className="flex items-center gap-1.5"
              style={{ color: 'var(--color-red)' }}
            >
              Account recovery
            </span>
          </BBChip>
        </div>

        {/* Hero */}
        <div className="px-7 pt-7 pb-5 bg-paper-2">
          <div className="flex gap-3.5 items-start">
            <div className="w-11 h-11 shrink-0 rounded-xl flex items-center justify-center bg-red-bg border border-red-border text-red">
              <Icon name="key" size={20} />
            </div>
            <div className="flex-1">
              <h1 className="text-[22px] font-semibold text-ink mb-2 leading-tight">
                We can&apos;t recover this for you.
              </h1>
              <p className="text-[13.5px] text-ink-2 leading-relaxed">
                Your recovery phrase is the only key to the vault. We never
                stored a copy. Not a backup, not a reset link, not a support
                override. That&apos;s the tradeoff you chose when you signed
                up for zero-knowledge encryption — and it&apos;s what keeps
                everyone else out.
              </p>
            </div>
          </div>
        </div>

        {/* Recovery options */}
        <div className="px-7 py-5">
          <div className="text-[10px] font-medium uppercase tracking-wider text-ink-3 mb-3">
            What you can still try
          </div>

          {recoveryOptions.map((opt, i) => (
            <div
              key={i}
              className="flex gap-3 py-3 border-b border-line"
            >
              <div className="w-7 h-7 shrink-0 rounded-lg flex items-center justify-center bg-green-bg border border-green-border text-green">
                <Icon name={opt.icon} size={13} />
              </div>
              <div className="flex-1">
                <div className="text-[13px] font-semibold mb-0.5">
                  {opt.title}
                </div>
                <div className="text-xs text-ink-3 leading-relaxed">
                  {opt.description}
                </div>
              </div>
              {opt.cta && (
                <BBButton
                  size="sm"
                  className="self-start shrink-0"
                  onClick={opt.onAction}
                >
                  {opt.cta}
                </BBButton>
              )}
            </div>
          ))}

          {children}

          {/* Dead-end: Start over */}
          <div className="mt-[18px] p-3.5 rounded-md bg-red-bg border border-red-border">
            <div className="flex items-center gap-2 mb-1.5">
              <Icon name="trash" size={13} className="text-red" />
              <span className="text-[13px] font-semibold text-red">
                Start over
              </span>
            </div>
            <p className="text-xs text-ink-2 leading-relaxed mb-2.5">
              If none of the above works, your files are lost —
              mathematically, not administratively. We can wipe the
              encrypted blobs from our servers and give you a new account
              with the same email.
            </p>
            <div className="flex gap-2">
              <BBButton size="sm" variant="danger" onClick={onStartOver}>
                Shred data &amp; create new account
              </BBButton>
              {onFoundPhrase && (
                <BBButton size="sm" variant="ghost" onClick={onFoundPhrase}>
                  I found my phrase
                </BBButton>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
