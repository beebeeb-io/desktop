// How shared components draw an in-app link without importing a router. Web passes a react-router
// Link; desktop passes its own navigator. The default is a plain anchor.
import type { ReactNode } from 'react'

export interface DriveLinkProps {
  to: string
  className?: string
  children: ReactNode
}

export type DriveLinkRenderer = (props: DriveLinkProps) => ReactNode

export function DefaultDriveLink({ to, className, children }: DriveLinkProps) {
  return (
    <a href={to} className={className}>
      {children}
    </a>
  )
}
