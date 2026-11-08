// Owned by batch B10. Signed out, locked or without a plan, the window shows sign-in (DW-A02), the lock
// (DW-0009) or no-plan (DW-0014) instead of Files. The scaffold lets every state through so frames build.
import type { ReactNode } from 'react'

export function AuthGate({ children }: { children: ReactNode }) {
  return <>{children}</>
}
