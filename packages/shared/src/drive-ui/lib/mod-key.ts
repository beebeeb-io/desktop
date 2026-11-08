// The platform's modifier key for list shortcuts (Cmd on macOS, Ctrl elsewhere).
const isMac = typeof navigator !== 'undefined' && /Mac|iPod|iPhone|iPad/.test(navigator.userAgent)
export const modKey = isMac ? 'metaKey' : 'ctrlKey'
