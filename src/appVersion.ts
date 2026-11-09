/** `app_version` is `{ version, release }` once Rust knows. A bare string is a dev build, never a release. */
export function appVersionFields(value: unknown): { version: string; release: boolean } | null {
  if (typeof value === 'string' && value.length > 0) return { version: value, release: false }
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const version = (value as { version?: unknown }).version
    const release = (value as { release?: unknown }).release
    if (typeof version === 'string' && version.length > 0) return { version, release: release === true }
  }
  return null
}
