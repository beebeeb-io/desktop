// B14 registers the Account and Devices panes here (one file, theirs).
import type { ComponentType } from 'react'
import type { PaneId } from './shell/context'

export const PANES_ACCOUNT: Partial<Record<PaneId, ComponentType>> = {}
