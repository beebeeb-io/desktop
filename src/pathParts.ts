/**
 * Path parts the windows show (2409 review P3: the tray, Files and Activity each had their own copy).
 */

/** The last part of a path, with either separator (a backup's path is a Windows one); a trailing one is ignored. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : path
}
