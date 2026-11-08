// Backups (DW-0422 to DW-0425): every device's backup and every Beebeeb service, in one protected folder. Owned by batch B04.
import type { SectionProps } from '../types'
import { FolderView } from '../files/FolderView'

export default function Section({ nav }: SectionProps) {
  return <FolderView nav={nav} mode="backups" />
}
