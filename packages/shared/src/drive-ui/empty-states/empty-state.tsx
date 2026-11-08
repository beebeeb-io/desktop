// The shared empty-state layout (icon, title, action) the views build on.
import type { ReactNode } from 'react'
import { Icon, type IconName } from '../../design'
import { BBButton } from '../../design'

interface EmptyStateProps {
  /** Icon shown inside the rounded container */
  icon: IconName
  /** Primary heading */
  heading: string
  /** Descriptive subtitle below the heading */
  subtitle: string
  /** Primary CTA button */
  cta?: {
    label: string
    icon?: IconName
    onClick: () => void
    variant?: 'amber' | 'default'
  }
  /** Optional secondary CTA */
  secondaryCta?: {
    label: string
    icon?: IconName
    onClick: () => void
  }
  /** Optional hint text below the CTA buttons */
  hint?: string
  /** Optional children rendered below everything */
  children?: ReactNode
}

/**
 * Shared empty state layout used across all pages.
 * Every empty state should have: icon, heading, subtitle, and at least one CTA.
 */
export function EmptyState({
  icon,
  heading,
  subtitle,
  cta,
  secondaryCta,
  hint,
  children,
}: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center h-full text-center py-20">
      <div
        className="w-14 h-14 mb-4 rounded-2xl flex items-center justify-center"
        style={{
          background: 'var(--color-amber-bg)',
          border: '1.5px solid var(--color-line-2)',
        }}
      >
        <Icon name={icon} size={24} className="text-amber-deep" />
      </div>

      <h2 className="text-[15px] font-semibold text-ink mb-1.5">{heading}</h2>

      <p className="text-[13px] text-ink-3 max-w-[340px] leading-relaxed mb-5">
        {subtitle}
      </p>

      {(cta || secondaryCta) && (
        <div className="flex gap-2 items-center">
          {cta && (
            <BBButton
              size="md"
              variant={cta.variant ?? 'amber'}
              onClick={cta.onClick}
              className="gap-1.5"
            >
              {cta.icon && <Icon name={cta.icon} size={13} />}
              {cta.label}
            </BBButton>
          )}
          {secondaryCta && (
            <BBButton
              size="md"
              variant="ghost"
              onClick={secondaryCta.onClick}
              className="gap-1.5"
            >
              {secondaryCta.icon && <Icon name={secondaryCta.icon} size={13} />}
              {secondaryCta.label}
            </BBButton>
          )}
        </div>
      )}

      {hint && (
        <p className="mt-3 text-[11px] text-ink-4 max-w-[300px]">{hint}</p>
      )}

      {children}
    </div>
  )
}
