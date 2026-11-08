// The shared BBChip design-system chip.
import type { ReactNode } from 'react'

type Variant = 'default' | 'amber' | 'green' | 'red'

interface BBChipProps {
  variant?: Variant
  className?: string
  children: ReactNode
}

const variantClasses: Record<Variant, string> = {
  default: 'bg-paper-2 text-ink-2 border border-line',
  amber: 'bg-amber-bg text-amber-deep',
  green: 'bg-green/10 text-green',
  red: 'bg-red/10 text-red',
}

export function BBChip({ variant = 'default', className = '', children }: BBChipProps) {
  return (
    <span
      className={`inline-flex items-center px-sm py-xs text-xs font-medium rounded-sm ${variantClasses[variant]} ${className}`}
    >
      {children}
    </span>
  )
}
