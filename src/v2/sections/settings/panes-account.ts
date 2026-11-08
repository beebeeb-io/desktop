// B14 registers the Account and Devices panes here (one file, theirs). Security is replaced too: B14's pane carries
// Drive's App lock (the securityExtra slot) above the shared sign-in rows, with every row working (DW-A52).
import type { ComponentType } from 'react'
import type { PaneId } from './shell/context'
import { AccountPane } from './account/AccountPane'
import { DevicesPane } from './account/DevicesPane'
import { SecurityPane } from './account/SecurityPane'

export const PANES_ACCOUNT: Partial<Record<PaneId, ComponentType>> = {
  account: AccountPane,
  security: SecurityPane,
  devices: DevicesPane,
}
