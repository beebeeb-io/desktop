// The Files toolbar as a gate draws it behind the sign-in, the lock and no plan (dimmed, DW-A02, DW-0009, DW-0014), and
// the frames draw it in their windows: the section's name and its New button, nothing that reads data.
import { BBButton } from '@beebeeb/shared/design'

export function Toolbar() {
  return (
    <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
      <h1 className="m-0 text-[15px] font-semibold text-ink">Files</h1>
      <span className="flex-1" />
      <BBButton variant="amber" size="md">New</BBButton>
    </div>
  )
}
