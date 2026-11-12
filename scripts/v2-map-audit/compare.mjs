// The M4 overlay rule (v2-m3c ruling item 2, v2-m3d ruling item 2): a dialog or message the target frame draws over a
// different section is compared on its dialog layer alone, never on the screen behind it. Pure and browser-free so the
// rule is testable where the Playwright run cannot be (tests/v2MapAuditCompare.test.ts).

/** The `dlg:` tokens of a frame signature: what the target draws in its open dialog or message layer. */
export function dialogLayer(sig) {
  return sig.filter((t) => t.startsWith('dlg:'))
}

/** The overlay rule applies when the target frame's section differs from the source frame's. */
export function isOverlaySection(sourceSection, targetSection) {
  return sourceSection !== targetSection
}
