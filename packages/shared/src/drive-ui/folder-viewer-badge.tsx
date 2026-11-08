// The compact folder-row viewer badge (stacked avatar dots + count). Presentational only.

// Props for the compact folder-row badge
interface FolderViewerBadgeProps {
  /** Number of collaborators who have access (excluding owner). */
  count: number
  /** Representative emails to colour-code the mini-avatars. */
  emails?: string[]
}

// ─── Colour palette for avatars ───────────────────────────────────────────────
// Six amber/neutral tones that stay on-brand. Colour is derived from the
// email string so it's stable across renders.

export const AVATAR_COLOURS: [bg: string, text: string][] = [
  ['#FEF3C7', '#92400E'], // amber-100 / amber-800
  ['#FDE68A', '#78350F'], // amber-200 / amber-900
  ['#FCD34D', '#451A03'], // amber-300 / amber-950
  ['#F0F0F0', '#404040'], // neutral-100 / neutral-700
  ['#E5E7EB', '#374151'], // gray-200 / gray-700
  ['#D1FAE5', '#065F46'], // emerald-100 / emerald-800
]

export function colourFor(email: string): [string, string] {
  let hash = 0
  for (let i = 0; i < email.length; i++) {
    hash = (hash * 31 + email.charCodeAt(i)) >>> 0
  }
  return AVATAR_COLOURS[hash % AVATAR_COLOURS.length]
}

// ─── FolderViewerBadge ───────────────────────────────────────────────────────
// Compact badge used inside file-list rows for shared folders.
// Shows up to 3 stacked avatar dots + a count when > 3.

export function FolderViewerBadge({ count, emails = [] }: FolderViewerBadgeProps) {
  if (count <= 0) return null

  const MAX_DOTS = 3
  const visibleEmails = emails.slice(0, MAX_DOTS)
  const overflow = count - MAX_DOTS

  return (
    <span
      className="inline-flex items-center gap-0.5 ml-1"
      title={`${count} collaborator${count !== 1 ? 's' : ''} have access`}
      aria-label={`${count} collaborator${count !== 1 ? 's' : ''}`}
    >
      {/* Stacked mini-circles */}
      <span className="flex -space-x-0.5">
        {visibleEmails.map((email, i) => {
          const [bg, fg] = colourFor(email)
          return (
            <span
              key={i}
              className="w-[12px] h-[12px] rounded-full ring-1 ring-paper inline-block shrink-0"
              style={{ background: bg, color: fg }}
            />
          )
        })}
        {/* Fallback dot when no emails are provided */}
        {visibleEmails.length === 0 && (
          <span className="w-[12px] h-[12px] rounded-full ring-1 ring-paper bg-paper-3 inline-block shrink-0" />
        )}
      </span>
      {/* Count label */}
      <span className="font-mono text-[9px] text-ink-3 tabular-nums leading-none">
        {count > MAX_DOTS && overflow > 0 ? `+${overflow}` : ''}
      </span>
    </span>
  )
}
