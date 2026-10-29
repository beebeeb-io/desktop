/**
 * Every string of the macOS sign-out flow (spec 2026-10-10 §11): the send phase (P), the ask (A), the kept folder (K)
 * and the errors (E). Each constant equals the one of the same name in `src-tauri/src/sign_out/copy.rs`, character for
 * character; tests/signOutCopy.test.ts reads every one from the Rust source and pins the two equal. Typographic
 * apostrophes, the one-character ellipsis, no emoji. The builders below fill `{n}`, `{c}`, `{k}` and `{m}`, and a
 * sentence whose count would be 0 is left out (null), never shown.
 */

export const E1_UNSENT_CHANGES = 'Some changes haven’t reached the server yet. Sign out from Settings › Account to send or keep them first.'
export const E2_UNSENT_COPY_FAILED = 'Beebeeb couldn’t copy the changes that haven’t reached the server, so it didn’t sign out. They’re still waiting to upload. Try again; if it keeps happening, restart Beebeeb.'
export const E3_UNSENT_COUNT_FAILED = 'Beebeeb couldn’t check for changes that haven’t reached the server, so it changed nothing. Try again. If it keeps happening, restart Beebeeb.'

export const P0_TITLE = 'Sending changes first'
export const P1_ONE = 'Sending 1 change before signing out…'
export const P1_MANY = 'Sending {n} changes before signing out…'
export const P2_HINT = 'This takes at most a minute.'
export const P3_STOP = 'Stop waiting'

export const A0_TITLE = 'Sign out with changes not sent?'
export const A0S_TITLE = 'Switch accounts with changes not sent?'
export const A1S_LINE = 'This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.'
export const A2_ONE = '1 file you changed in Finder hasn’t reached the server.'
export const A2_MANY = '{n} files you changed in Finder haven’t reached the server.'
export const A2_COPY_IT = 'Before signing out, Beebeeb copies it to a folder on this Mac and shows you where.'
export const A2_COPY_THEM = 'Before signing out, Beebeeb copies them to a folder on this Mac and shows you where.'
export const A2_COPY_SOME = 'Before signing out, Beebeeb copies {c} of them to a folder on this Mac and shows you where.'
export const A2S_ONE = '1 file changed in Finder while that account was signed in hasn’t reached the server.'
export const A2S_MANY = '{n} files changed in Finder while that account was signed in haven’t reached the server.'
export const A2S_COPY_IT = 'Before switching, Beebeeb copies it to a folder on this Mac and shows you where.'
export const A2S_COPY_THEM = 'Before switching, Beebeeb copies them to a folder on this Mac and shows you where.'
export const A2S_COPY_SOME = 'Before switching, Beebeeb copies {c} of them to a folder on this Mac and shows you where.'
export const A3_ONE_OF_ONE = 'Beebeeb’s copy of a change to that file is missing, so that change can’t be kept.'
export const A3_ONE_OF_MANY = 'Beebeeb’s copy of a change to 1 of these files is missing, so that change can’t be kept.'
export const A3_MANY = 'Beebeeb’s copies of changes to {k} of these files are missing, so those changes can’t be kept.'
export const A3E_ONE = 'A sign-out that didn’t finish copied changes to this folder, so a change listed as missing may be there:'
export const A3E_MANY = 'Sign-outs that didn’t finish copied changes to these folders, so a change listed as missing may be in one of them:'
export const A4_ONE = '1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server{either}. Signing out drops it, and that item stays as it is on the server.'
export const A4_MANY = '{m} other changes (renames, moves, deletions, new folders or restores) haven’t reached the server{either}. Signing out drops them, and those items stay as they are on the server.'
export const A4S_ONE = '1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server{either}. Switching drops it, and that item stays as it is on the server.'
export const A4S_MANY = '{m} other changes (renames, moves, deletions, new folders or restores) haven’t reached the server{either}. Switching drops them, and those items stay as they are on the server.'
export const A5_OFFLINE = 'This Mac can’t reach the server right now.'
export const A5_SESSION = 'The server no longer accepts this Mac’s sign-in.'
export const A5_PAUSED = 'Sync is paused.'
export const A5_HELD = 'These are on hold after earlier failures.'
export const A5_ENGINE_STOPPED = 'Sync stopped before everything was sent.'
export const A5_TIME_LIMIT = 'Sending took longer than a minute. Choose Cancel to let it finish, then sign out again.'
export const A5_ACCOUNT = 'Beebeeb can’t accept uploads for this account until it has free space or an active plan.'
export const ASK_CANCEL = 'Cancel'
export const ASK_SIGN_OUT = 'Sign out'
export const ASK_SWITCH = 'Sign out and switch'

export const K0_TITLE = 'Files kept on this Mac'
export const K1_ONE = 'Beebeeb copied 1 file with changes that hadn’t reached the server to this folder on this Mac:'
export const K1_MANY = 'Beebeeb copied {n} files with changes that hadn’t reached the server to this folder on this Mac:'
export const K2_ROW = 'Beebeeb copied changes that had not reached the server to this folder:'
export const K2_SHOW = 'Show in Finder'
export const K2_DISMISS = 'Dismiss'
export const K3_TOAST = 'That folder isn’t on this Mac any more. It may have been moved or deleted.'

export type NotSending = 'engine_stopped' | 'session' | 'offline' | 'paused' | 'held'
export type EndReason = 'all_sent' | 'held' | 'time_limit' | 'stopped' | 'closed' | 'offline' | 'session' | 'engine_stopped' | 'account'
export interface AskCounts { askId: string; files: number; copiedFiles: number; others: number; missingFiles: number; earlierFolders: string[] }
export interface SignOutCount extends AskCounts { canSend: boolean; notSending: NotSending | null }
export interface SendEnded { reason: EndReason; ask: AskCounts | null }

const fill = (template: string, key: string, value: number) => template.replace(key, String(value))

/** P1, never for 0 (W2). */
export function progressLine(n: number): string | null {
  if (n <= 0) return null
  return n === 1 ? P1_ONE : fill(P1_MANY, '{n}', n)
}

/** A2 or A2s: the count, then the copy sentence by c; left out for c = 0. */
export function filesLine(n: number, c: number, isSwitch: boolean): string | null {
  if (n <= 0) return null
  const [one, many, it, them, some] = isSwitch
    ? [A2S_ONE, A2S_MANY, A2S_COPY_IT, A2S_COPY_THEM, A2S_COPY_SOME]
    : [A2_ONE, A2_MANY, A2_COPY_IT, A2_COPY_THEM, A2_COPY_SOME]
  const first = n === 1 ? one : fill(many, '{n}', n)
  if (c <= 0) return first
  const second = c === n ? (n === 1 ? it : them) : fill(some, '{c}', c)
  return `${first} ${second}`
}

export function missingLine(n: number, k: number): string | null {
  if (k <= 0) return null
  if (k === 1) return n === 1 ? A3_ONE_OF_ONE : A3_ONE_OF_MANY
  return fill(A3_MANY, '{k}', k)
}

export function earlierFoldersLine(count: number): string | null {
  if (count <= 0) return null
  return count === 1 ? A3E_ONE : A3E_MANY
}

export function othersLine(m: number, either: boolean, isSwitch: boolean): string | null {
  if (m <= 0) return null
  const [one, many] = isSwitch ? [A4S_ONE, A4S_MANY] : [A4_ONE, A4_MANY]
  return (m === 1 ? one : fill(many, '{m}', m)).replace('{either}', either ? ' either' : '')
}

export function reasonLine(reason: EndReason | NotSending | null): string | null {
  switch (reason) {
    case 'offline': return A5_OFFLINE
    case 'session': return A5_SESSION
    case 'paused': return A5_PAUSED
    case 'held': return A5_HELD
    case 'engine_stopped': return A5_ENGINE_STOPPED
    case 'time_limit': return A5_TIME_LIMIT
    case 'account': return A5_ACCOUNT
    default: return null
  }
}

export function keptAlertLine(n: number): string | null {
  if (n <= 0) return null
  return n === 1 ? K1_ONE : fill(K1_MANY, '{n}', n)
}

/** The ask's body, in order: A2, A3, A4, A5 (the switch shows no reason line). A3e's sentence and paths go under A3:
 *  after the first `foldersAfter` paragraphs. */
export function askLines(ask: AskCounts, reason: EndReason | NotSending | null, isSwitch: boolean) {
  const files = filesLine(ask.files, ask.copiedFiles, isSwitch)
  const head = [files, missingLine(ask.files, ask.missingFiles)].filter((line): line is string => line !== null)
  const tail = [othersLine(ask.others, files !== null, isSwitch), isSwitch ? null : reasonLine(reason)].filter(
    (line): line is string => line !== null,
  )
  const folders = ask.missingFiles > 0 ? ask.earlierFolders : []
  return { paragraphs: [...head, ...tail], foldersAfter: head.length, folders, foldersLine: earlierFoldersLine(folders.length) }
}

/** §11: a caller matches the code; a person reads the sentence. */
export function stripCode(message: string): string {
  return message.replace(/^(unsent_changes|unsent_copy_failed|unsent_count_failed): /, '')
}
