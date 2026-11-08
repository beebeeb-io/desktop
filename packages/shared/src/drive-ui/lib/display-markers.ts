// Display markers derived from what THIS client has proven, never from
// server-supplied attribution columns (spec 2275, "Display MUST").
//
// - An F item is marked from the id class core answered for its id
//   (`classifyItemIds`), not from `uploaded_by` or `acted_via_share_id`.
// - A shared root is marked from the client's own custody proof by id
//   (`hasSeenCustody`), never from a name.

interface Proofs {
  sharedRoot?: (id: string) => boolean
  foreign?: (id: string) => boolean
}
const proofs: Proofs = {}

/** The host plugs in what THIS client has proven (web: custody memory and owner adoption). */
export function registerDisplayProofs(p: Proofs): void {
  proofs.sharedRoot = p.sharedRoot
  proofs.foreign = p.foreign
}

/** The marker an F row carries. Never a member name or a link label. */
export const FOREIGN_ROW_LABEL = 'Added by someone else'

/** Suffix that marks a shared root in a crumb or a picker row. */
export const SHARED_ROOT_SUFFIX = ' (shared)'

/** True when this client has proven custody of `id` (a shared root it owns or joined). */
export function isSharedRootById(id: string | null | undefined): boolean {
  if (!id || !proofs.sharedRoot) return false
  return proofs.sharedRoot(id)
}

/** The suffix a crumb or picker label for `id` carries, '' when not a proven root. */
export function sharedRootSuffix(id: string | null | undefined): string {
  return isSharedRootById(id) ? SHARED_ROOT_SUFFIX : ''
}

/** The label a picker shows for a folder: its name, marked when it is a shared root. */
export function pickerFolderLabel(id: string, name: string): string {
  return `${name}${sharedRootSuffix(id)}`
}

/**
 * The marker an F row shows, from the id class this client holds. A row that
 * has not been classified is not marked: the class is the only authority.
 */
export function foreignRowMarker(itemId: string): string | null {
  return proofs.foreign?.(itemId) ? FOREIGN_ROW_LABEL : null
}

export interface CrumbItem {
  id: string | null
  name: string
}

export type DisplayCrumb =
  | { kind: 'crumb'; id: string | null; name: string; shared: boolean }
  | { kind: 'ellipsis' }

/** Above this many crumbs the trail collapses. */
export const BREADCRUMB_MAX_VISIBLE = 4

/**
 * The trail to draw. When it is too long, the root, the proven shared root and
 * the current segment stay visible with an ellipsis between them: a collapsed
 * trail never drops the shared root or the place you are ("Drive > Family
 * (shared) > … > 2024"). Marking is by id only.
 */
export function visibleCrumbs(
  items: readonly CrumbItem[],
  options: { max?: number; isSharedRoot?: (id: string | null) => boolean } = {},
): DisplayCrumb[] {
  const max = options.max ?? BREADCRUMB_MAX_VISIBLE
  const isSharedRoot = options.isSharedRoot ?? isSharedRootById
  const asCrumb = (item: CrumbItem): DisplayCrumb => ({
    kind: 'crumb',
    id: item.id,
    name: item.name,
    shared: isSharedRoot(item.id),
  })

  if (items.length <= max) return items.map(asCrumb)

  const first = items[0]
  const last = items[items.length - 1]
  const middle = items.slice(1, -1)
  const sharedRoot = middle.find((item) => isSharedRoot(item.id))

  const kept: DisplayCrumb[] = [asCrumb(first)]
  if (sharedRoot && sharedRoot.id !== first.id) kept.push(asCrumb(sharedRoot))
  kept.push({ kind: 'ellipsis' })
  kept.push(asCrumb(last))
  return kept
}
