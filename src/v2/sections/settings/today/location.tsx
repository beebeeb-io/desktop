// General › Storage location (DW-0788 family): where new files are stored, inside the EU. Countries only.
// No command reads or writes the account's storage location in this build (0 hits for storage_region/storage_location in
// src-tauri), so this page says so and names what is missing instead of drawing a country list that saves nothing.
import { Page } from '../shell/kit'
import { BackLink, InfoBanner } from './ui'

/** Kept so `Location` and the frames type-check; a frame that needs the board's list gets it in M4's board-data pass. */
export interface LocationProps { chosen?: string; strict?: boolean; unavailable?: boolean; error?: boolean; toast?: boolean }

export function Location(_props: LocationProps = {}) {
  return (
    <Page title="Storage location" sub="Choose where new files are stored, inside the EU." back={<BackLink to="general">General</BackLink>}>
      <InfoBanner title="The storage location can't be chosen in this build yet.">
        Your files stay in the EU. Choosing a country needs the <span className="font-mono">get_storage_location</span> and{' '}
        <span className="font-mono">set_storage_location</span> commands, which this build does not have yet. Countries only, inside the EU.
      </InfoBanner>
    </Page>
  )
}
