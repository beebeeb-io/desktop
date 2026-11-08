// The drive breadcrumb trail with folder navigation and overflow collapse.
import { Icon } from '../design'
import { SHARED_ROOT_SUFFIX, visibleCrumbs, type CrumbItem } from './lib/display-markers'

export interface BreadcrumbItem extends CrumbItem {}

export interface BreadcrumbProps {
  items: BreadcrumbItem[]
  onNavigate: (index: number) => void
}

/**
 * Draws the trail. A collapsed trail keeps the shared root and the current
 * segment (spec 2275, A17i); both the collapse and the "(shared)" marker are
 * decided by id from this client's own proof (`visibleCrumbs`).
 */
export function Breadcrumb({ items, onNavigate }: BreadcrumbProps) {
  const shown = visibleCrumbs(items)
  return (
    <div className="flex items-center gap-1.5 text-sm" data-testid="breadcrumb">
      {shown.map((crumb, i) => {
        if (crumb.kind === 'ellipsis') {
          return (
            <span key="ellipsis" className="flex items-center gap-1.5" aria-hidden="true">
              <Icon name="chevron-right" size={12} className="text-ink-4" />
              <span className="text-ink-4" data-testid="breadcrumb-ellipsis">
                …
              </span>
            </span>
          )
        }
        const isLast = i === shown.length - 1
        // The index into the full trail, so onNavigate still means "the crumb
        // with this id" after collapsing.
        const target = items.findIndex((item) => item.id === crumb.id && item.name === crumb.name)
        const label = crumb.shared ? `${crumb.name}${SHARED_ROOT_SUFFIX}` : crumb.name
        return (
          <span
            key={crumb.id ?? 'root'}
            className={`flex items-center gap-1.5${isLast && i > 0 ? ' breadcrumb-enter' : ''}`}
          >
            {i > 0 && <Icon name="chevron-right" size={12} className="text-ink-4" />}
            {isLast ? (
              <span
                className="font-semibold text-ink"
                title={crumb.shared ? 'Shared folder' : undefined}
                aria-label={crumb.shared ? `${crumb.name}, shared folder` : undefined}
              >
                {label}
              </span>
            ) : (
              <button
                onClick={() => onNavigate(target)}
                className="text-ink-3 hover:text-ink transition-colors cursor-pointer"
                title={crumb.shared ? 'Shared folder' : undefined}
                aria-label={crumb.shared ? `${crumb.name}, shared folder` : undefined}
              >
                {label}
              </button>
            )}
          </span>
        )
      })}
    </div>
  )
}
