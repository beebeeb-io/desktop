// M2: the sidebar counts re-read on drive events through `throttled`, so a sync's burst of events never floods share_list.
import { describe, expect, test } from 'bun:test'
import { throttled } from '../src/v2/throttle'

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

describe('throttled', () => {
  test('the first call runs at once; a burst inside the window becomes one run at its end', async () => {
    let runs = 0
    const t = throttled(() => { runs++ }, 40)
    t()
    expect(runs).toBe(1)
    for (let i = 0; i < 20; i++) t()
    expect(runs).toBe(1)
    await wait(70)
    expect(runs).toBe(2)
    await wait(70)
    expect(runs).toBe(2)
  })

  test('a call after a quiet window runs at once again', async () => {
    let runs = 0
    const t = throttled(() => { runs++ }, 30)
    t()
    await wait(60)
    t()
    expect(runs).toBe(2)
    t.cancel()
  })

  test('cancel drops the pending run', async () => {
    let runs = 0
    const t = throttled(() => { runs++ }, 30)
    t()
    t()
    t.cancel()
    await wait(60)
    expect(runs).toBe(1)
  })
})
