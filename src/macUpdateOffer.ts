/**
 * What a Mac is offered for an update it found, by channel (task 2176; PR #140 review, P2-4).
 *
 * `MACOS_DOWNLOAD_URL` is the site's redirect to the newest STABLE Mac disk image. It is the right link for a stable update and
 * the wrong one for a beta or an alpha: the person would download another build than the one that was offered. So a stable update
 * links there, and any other channel links to the release page of the version that channel advertises and says which channel it
 * is. A channel nobody recognises is not taken for stable.
 */
import { MACOS_DOWNLOAD_URL, UPDATE_DOWNLOAD_HINT, UPDATE_DOWNLOAD_LABEL } from './macUpdateCopy'

const RELEASE_PAGE = 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v'

/** The release page of a version: the same address the manifest's `release_notes_url` carries. */
export function releasePageUrl(version: string): string {
  return `${RELEASE_PAGE}${encodeURIComponent(version)}`
}

export interface UpdateOffer {
  label: string
  url: string
  /** The sentence that says what to do with it. */
  hint: string
  /** True when the link is the newest stable disk image, which is what a stable update is. */
  stableBuild: boolean
}

export function updateOffer(channel: string | null | undefined, version: string): UpdateOffer {
  if (channel === 'stable') return { label: UPDATE_DOWNLOAD_LABEL, url: MACOS_DOWNLOAD_URL, hint: UPDATE_DOWNLOAD_HINT, stableBuild: true }
  const name = channel === 'beta' ? 'Beta' : channel === 'alpha' ? 'Alpha' : null
  const which = name === null ? 'a' : name === 'Alpha' ? 'an Alpha' : 'a Beta'
  return {
    label: 'View release',
    url: releasePageUrl(version),
    hint: `This is ${which} release. Download its Mac disk image from the release page, then drag Beebeeb into Applications to replace this version.`,
    stableBuild: false,
  }
}
