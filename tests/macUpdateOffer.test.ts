/**
 * What a Mac is offered for the update it found, by channel (PR #140 review, P2-4). `https://beebeeb.io/download/macos-dmg`
 * serves the newest STABLE release: it is the right link for a stable update and the wrong one for a beta or an alpha, so
 * for those the offer is the release page of the version the channel advertises, and it says so. A link is never allowed to
 * imply it is the version that was offered.
 */
import { describe, expect, test } from 'bun:test'
import { MACOS_DOWNLOAD_URL, UPDATE_DOWNLOAD_HINT, UPDATE_DOWNLOAD_LABEL } from '../src/macUpdateCopy'
import { releasePageUrl, updateOffer } from '../src/macUpdateOffer'

describe('the update a Mac is offered', () => {
  test('stable: the newest Mac disk image, with the one sentence that says what to do', () => {
    expect(updateOffer('stable', '0.8.13')).toEqual({ label: UPDATE_DOWNLOAD_LABEL, url: MACOS_DOWNLOAD_URL, hint: UPDATE_DOWNLOAD_HINT, stableBuild: true })
  })

  test('beta and alpha: the release page of that version, and a hint that names the channel and does not pretend', () => {
    const beta = updateOffer('beta', '0.9.0-beta.2')
    expect(beta).toEqual({
      label: 'View release',
      url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.9.0-beta.2',
      hint: 'This is a Beta release. Download its Mac disk image from the release page, then drag Beebeeb into Applications to replace this version.',
      stableBuild: false,
    })
    const alpha = updateOffer('alpha', '0.9.1-alpha.1')
    expect(alpha.url).toBe('https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.9.1-alpha.1')
    expect(alpha.hint).toContain('This is an Alpha release.')
    for (const offer of [beta, alpha]) {
      expect(offer.url).not.toBe(MACOS_DOWNLOAD_URL)
      expect(offer.label).not.toBe(UPDATE_DOWNLOAD_LABEL)
    }
  })

  test('a channel that is not known is not stable: it gets the release page, never the stable link', () => {
    for (const channel of [undefined, null, 'nightly'] as const) {
      const offer = updateOffer(channel as never, '1.2.3')
      expect(offer.url, String(channel)).toBe('https://github.com/beebeeb-io/desktop/releases/tag/desktop-v1.2.3')
      expect(offer.stableBuild).toBe(false)
    }
  })

  test('the release page is the one the manifest names: the repository, the desktop-v tag, the version URL-encoded', () => {
    expect(releasePageUrl('0.8.13')).toBe('https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.13')
    expect(releasePageUrl('1.0.0+build 5')).toBe('https://github.com/beebeeb-io/desktop/releases/tag/desktop-v1.0.0%2Bbuild%205')
  })
})
