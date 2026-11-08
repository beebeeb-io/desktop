// B13 registers the panes for what the app does today here (one file, theirs).
import type { ComponentType } from 'react'
import type { PaneId } from './shell/context'

export const PANES_TODAY: Partial<Record<PaneId, ComponentType>> = {}
