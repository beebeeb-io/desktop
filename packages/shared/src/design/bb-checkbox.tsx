// The shared BBCheckbox design-system control.
import { useId } from 'react'
import { Icon } from './icons'

interface BBCheckboxProps {
  checked: boolean
  indeterminate?: boolean
  onChange: (checked: boolean) => void
  label?: string
  id?: string
  className?: string
}

export function BBCheckbox({
  checked,
  indeterminate,
  onChange,
  label,
  id: externalId,
  className = '',
}: BBCheckboxProps) {
  const autoId = useId()
  const id = externalId ?? autoId

  const isActive = checked || indeterminate

  return (
    <label htmlFor={id} className={`flex items-start gap-2 cursor-pointer ${className}`}>
      <button
        id={id}
        type="button"
        role="checkbox"
        aria-checked={indeterminate ? 'mixed' : checked}
        onClick={() => onChange(!checked)}
        className={`mt-0.5 flex shrink-0 items-center justify-center w-4 h-4 rounded-sm border transition-all ${
          isActive
            ? 'bg-amber border-amber-deep'
            : 'bg-paper border-line-2'
        }`}
      >
        {indeterminate && !checked && (
          <span className="block w-2 h-0.5 bg-white rounded-full" />
        )}
        {checked && <Icon name="check" size={12} className="text-white" strokeWidth={2.5} />}
      </button>
      {label && (
        <span className="text-xs text-ink-2 leading-relaxed select-none">
          {label}
        </span>
      )}
    </label>
  )
}
