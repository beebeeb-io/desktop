// Dev only (P1c), never bundled: the board's example data a window with no host draws, registered through ../board.ts
// before the window mounts (./live.ts). Production modules ask ../board.ts by name and never import the fixtures; in a
// production build nothing is registered, so they show their honest empty or "unavailable" states.
import { registerBoard } from '../board'
import { scenarioFromSearch } from '../source/fixtures'
import { createBoardSource } from '../source/fixtures/board'
import { createFixtureExtras } from '../source/fixtures/extras'
import { FRAMES } from '../frames/registry'

export function installBoard(): void {
  registerBoard({
    source: (search: string) => createBoardSource(scenarioFromSearch(search)),
    extras: (scenario: string) => createFixtureExtras(scenario),
    // The frame table (`?frame=DW-xxxx`): the dev server's only, so the board's drawings never reach a production build.
    frames: FRAMES,
  })
}
