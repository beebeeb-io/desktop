// The shared BBInput design-system text input.
import { type InputHTMLAttributes, type ReactNode, useId } from 'react'
import { Icon } from './icons'
import type { IconName } from './icons'

interface BBInputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  label?: string
  hint?: string
  icon?: IconName
  trailing?: ReactNode
  error?: string
}

export function BBInput({
  label,
  hint,
  icon,
  trailing,
  error,
  className = '',
  id: externalId,
  ...props
}: BBInputProps) {
  const autoId = useId()
  const id = externalId ?? autoId

  const displayHint = error ?? hint

  return (
    <div className={className}>
      {label && (
        <label
          htmlFor={id}
          className="block text-xs font-medium text-ink-2 mb-1.5"
        >
          {label}
        </label>
      )}
      <div
        className={`flex items-center gap-2 border rounded-md bg-paper px-3 py-2 transition-all ${
          error
            ? 'border-red'
            : 'border-line focus-within:ring-2 focus-within:ring-amber/30 focus-within:border-amber-deep'
        }`}
      >
        {icon && (
          <Icon name={icon} size={16} className="shrink-0 text-ink-3" />
        )}
        <input
          id={id}
          className="flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-4"
          {...props}
        />
        {trailing}
      </div>
      {displayHint && (
        <p
          className={`text-xs mt-1.5 leading-relaxed ${
            error ? 'text-red' : 'text-ink-3'
          }`}
        >
          {displayHint}
        </p>
      )}
    </div>
  )
}
