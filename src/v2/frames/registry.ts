// Every batch's frames in one table. Each batch edits only its own bNN.tsx; this file never changes.
import type { FrameRegistry } from './types'
import { FRAMES as b03 } from './b03'
import { FRAMES as b04 } from './b04'
import { FRAMES as b05 } from './b05'
import { FRAMES as b06 } from './b06'
import { FRAMES as b07 } from './b07'
import { FRAMES as b08 } from './b08'
import { FRAMES as b09 } from './b09'
import { FRAMES as b10 } from './b10'
import { FRAMES as b11 } from './b11'
import { FRAMES as b12 } from './b12'
import { FRAMES as b13 } from './b13'
import { FRAMES as b14 } from './b14'

export const FRAMES: FrameRegistry = {
  ...b03,
  ...b04,
  ...b05,
  ...b06,
  ...b07,
  ...b08,
  ...b09,
  ...b10,
  ...b11,
  ...b12,
  ...b13,
  ...b14,
}
