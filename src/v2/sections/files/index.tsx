// Files (DW-0401 and on): the whole Drive in the window, from the copy on this PC. Owned by batch B04.
import type { SectionProps } from '../types'
import { FolderView } from './FolderView'

export default function Section({ nav }: SectionProps) {
  return <FolderView nav={nav} mode="files" />
}
