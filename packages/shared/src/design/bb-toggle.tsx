/**
 * The one switch, shared by web and admin.
 *
 * The <button> is a transparent hit box; the colour sits on the inner track,
 * never on the button itself, so on and off change only colours and the knob
 * position, never the box. (Admin used to restyle every `button.bg-amber`,
 * which stretched the old toggle only while it was on.)
 *
 * The hit box is larger than the track and is pulled back with negative
 * margins, so it takes the track's footprint in layout: placing a toggle next
 * to text looks the same as before, but the tap area is the bigger box.
 *
 *   md        track 36×20, hit box 52×36 (default; web's size)
 *   touch     track 44×26, hit box 60×44
 *   adaptive  touch below 768px, md from 768px (admin: "Component kit" artboard)
 */
type BBToggleSize = 'md' | 'touch' | 'adaptive'

interface BBToggleProps {
  on?: boolean
  onChange?: (on: boolean) => void
  disabled?: boolean
  size?: BBToggleSize
  'aria-label'?: string
}

const boxClasses: Record<BBToggleSize, string> = {
  md: 'h-9 w-[52px] -m-2',
  touch: 'h-11 w-[60px] -mx-2 -my-[9px]',
  adaptive: 'h-11 w-[60px] -mx-2 -my-[9px] md:h-9 md:w-[52px] md:-m-2',
}

const trackClasses: Record<BBToggleSize, string> = {
  md: 'h-5 w-9',
  touch: 'h-[26px] w-11',
  adaptive: 'h-[26px] w-11 md:h-5 md:w-9',
}

const knobClasses: Record<BBToggleSize, string> = {
  md: 'size-3.5',
  touch: 'size-5',
  adaptive: 'size-5 md:size-3.5',
}

// Knob travel = inner track width − knob − 2px gap, so it sits 2px from either end.
const knobOnClasses: Record<BBToggleSize, string> = {
  md: 'translate-x-[18px]',
  touch: 'translate-x-5',
  adaptive: 'translate-x-5 md:translate-x-[18px]',
}

export function BBToggle({
  on = false,
  onChange,
  disabled = false,
  size = 'md',
  'aria-label': ariaLabel,
}: BBToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange?.(!on)}
      className={`relative inline-flex shrink-0 items-center justify-center rounded-full align-middle cursor-pointer disabled:cursor-not-allowed disabled:opacity-45 ${boxClasses[size]}`}
    >
      <span
        aria-hidden="true"
        className={`relative flex items-center rounded-full border box-border transition-colors ${trackClasses[size]} ${
          on ? 'bg-amber border-amber-deep' : 'bg-paper-3 border-line-2'
        }`}
      >
        <span
          className={`inline-block rounded-full bg-white shadow-1 transition-transform ${knobClasses[size]} ${
            on ? knobOnClasses[size] : 'translate-x-0.5'
          }`}
        />
      </span>
    </button>
  )
}
