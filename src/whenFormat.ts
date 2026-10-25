/**
 * When something happened, in the words Review and Trash use (task 1683 slice 4; hi-fi section 3: "today 09:12",
 * "yesterday", "12 Oct"). Pure: `now` is passed in, and the calendar is the person's own (local midnight is the day
 * boundary, not 24 hours). English month names, because the whole window is English; sizes follow the Mac's locale
 * (`storageFormat.ts`), dates here do not.
 */

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] as const

const pad = (n: number) => String(n).padStart(2, '0')

/**
 * A time as the app receives it: seconds or milliseconds since the epoch (a number, or the same digits as a string,
 * which is how the Trash list arrives), or an ISO date. Anything that is not a time is null; so is the epoch itself,
 * which the engine uses for "never".
 */
export function parseWhen(value: string | number | null | undefined): Date | null {
  let n: number
  if (typeof value === 'number') {
    n = value
  } else if (typeof value === 'string' && value.trim() !== '') {
    const text = value.trim()
    n = /^\d+(\.\d+)?$/.test(text) ? Number(text) : Date.parse(text)
  } else {
    return null
  }
  if (!Number.isFinite(n) || n <= 0) return null
  // 10^10 seconds is the year 2286, 10^10 milliseconds is April 1970: a count above it is milliseconds.
  return new Date(n > 10_000_000_000 ? n : n * 1000)
}

const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()

/** "today", "yesterday", "12 Oct", or "12 Oct 2025" when it is not this year. */
export function dayLabel(date: Date, now: Date): string {
  if (sameDay(date, now)) return 'today'
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
  if (sameDay(date, yesterday)) return 'yesterday'
  const day = `${date.getDate()} ${MONTHS[date.getMonth()]}`
  return date.getFullYear() === now.getFullYear() ? day : `${day} ${date.getFullYear()}`
}

/** "09:12", 24-hour, the person's own clock. */
export function clockLabel(date: Date): string {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** "today 09:12", "yesterday 18:03", then the day. Null when the time is unknown (no placeholder). */
export function whenLabel(value: string | number | null | undefined, now: Date): string | null {
  const date = parseWhen(value)
  if (date === null) return null
  const day = dayLabel(date, now)
  return day === 'today' || day === 'yesterday' ? `${day} ${clockLabel(date)}` : day
}
