import type { Navigation } from '../routes'

/** What the shell hands every section. A section reads data only through useDriveSource(). */
export interface SectionProps {
  nav: Navigation
}
