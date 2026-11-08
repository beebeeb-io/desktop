// The two-letter country of a file's storage pool, never a city (LD-0021).
export function fileStorageCountry(file: unknown): string | null {
  if (!file || typeof file !== 'object') return null
  const loc = (file as { storage_location?: unknown }).storage_location
  if (!loc || typeof loc !== 'object') return null
  const country = (loc as { country?: unknown }).country
  if (typeof country !== 'string') return null
  const trimmed = country.trim()
  if (!trimmed || trimmed.toLowerCase() === 'unknown') return null
  return trimmed
}
