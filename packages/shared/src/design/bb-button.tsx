// The shared BBButton design-system button.
import type { ButtonHTMLAttributes } from 'react'

type Variant = 'amber' | 'default' | 'ghost' | 'danger'
type Size = 'sm' | 'md' | 'lg'

interface BBButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant
  size?: Size
}

const variantClasses: Record<Variant, string> = {
  amber: 'bg-amber text-[oklch(0.22_0.01_70)] hover:brightness-95 active:brightness-90',
  default: 'bg-paper text-ink border border-line hover:bg-paper-2 active:bg-paper-3',
  ghost: 'bg-transparent text-ink-2 hover:bg-paper-2 active:bg-paper-3',
  danger: 'bg-red text-white hover:brightness-95 active:brightness-90',
}

const sizeClasses: Record<Size, string> = {
  sm: 'px-sm py-xs text-sm rounded-sm',
  md: 'px-md py-sm text-sm rounded-md',
  lg: 'px-lg py-md text-base rounded-lg',
}

export function BBButton({
  variant = 'default',
  size = 'md',
  className = '',
  children,
  ...props
}: BBButtonProps) {
  return (
    <button
      className={`inline-flex items-center justify-center font-medium transition-all cursor-pointer focus-visible:ring-2 focus-visible:ring-amber-deep focus-visible:ring-offset-2 ${variantClasses[variant]} ${sizeClasses[size]} ${className}`}
      {...props}
    >
      {children}
    </button>
  )
}
