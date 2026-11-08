import { createContext, useContext, type ReactNode } from 'react'
import type { DriveSource } from './source'

const DriveSourceContext = createContext<DriveSource | null>(null)

export function DriveSourceProvider({ source, children }: { source: DriveSource; children: ReactNode }) {
  return <DriveSourceContext.Provider value={source}>{children}</DriveSourceContext.Provider>
}

/** The DriveSource of the nearest provider. A shared Drive component outside one is a bug, so this throws. */
export function useDriveSource(): DriveSource {
  const source = useContext(DriveSourceContext)
  if (!source) throw new Error('useDriveSource: no DriveSourceProvider above this component')
  return source
}
