/**
 * Task 1670 round 4 (Codex P1 on PR #75, review thread PRRT_kwDOSLX6Xs6ncqQ7
 * on `src-tauri/src/ipc_socket.rs:462`).
 *
 * Round 3 handed OUR OWN App-Group staging URL straight to `fetchContents`'s
 * `completionHandler` and relied on a 2-minute TTL sweep to eventually
 * delete it. Codex's review made the sharper point: once `completionHandler`
 * is called with a URL, "the File Provider contract transfers control of
 * that local copy to the system; there is no documented maximum delay
 * before the system finishes consuming it" — so deleting THAT SAME URL
 * later, on any timer however generous, can race a busy or suspended
 * `fileproviderd` and reopen this task's original "Couldn't communicate
 * with a helper application" bug.
 *
 * Round 4's fix: never hand our own staging URL to the system at all. Copy
 * it into `NSFileProviderManager(for: domain).temporaryDirectoryURL()` — a
 * SYSTEM-managed directory this daemon never touches — under a fresh name,
 * delete OUR staging copy immediately (we own it end to end; it was never
 * handed to anyone), and hand the COPY's URL to `completionHandler` instead.
 *
 * `BeebeebFileProvider` (the File Provider extension target) has no XCTest
 * target in this repo (checked again this round: still no
 * `.xcodeproj`/`.xctestplan`), so — same pattern as this repo's own
 * `noHardcodedThemeColors.test.ts`/`noAdHocErrorSurface.test.ts` guards, and
 * this file's own round-3 predecessor — this reads the real Swift source
 * text and pins the shape structurally:
 *
 * - the success (`do`) path must call a `copyToSystemTemporaryDirectory`
 *   helper and hand ITS RESULT to `completionHandler` — never
 *   `destinationURL` (our own App-Group staging file) directly.
 * - the success path must clean up the staging copy (`cleanupStagedPlaintext()`)
 *   AFTER the handoff copy succeeds — round 4 restores an eager delete on
 *   success, but of OUR OWN copy only, never the one handed to the system.
 * - every failure path (the `catch` block) must still call
 *   `cleanupStagedPlaintext()`.
 * - `copyToSystemTemporaryDirectory` itself must resolve
 *   `NSFileProviderManager(for: domain)` and call `.temporaryDirectoryURL()`
 *   — the system-managed, same-volume directory Apple's docs name
 *   specifically for `fetchContents` — and must clean up its own partial
 *   copy on any internal failure before rethrowing.
 *
 * Mutation check (2026-09-30, reverted after confirming RED each time — see
 * the task file's Notes for the pasted output):
 * 1. Removed the `copyToSystemTemporaryDirectory` call from the `do` block
 *    and passed `destinationURL` straight to `completionHandler` again
 *    (reintroducing round 3's exact bug) — the first assertion below failed.
 * 2. Moved `cleanupStagedPlaintext()` to run BEFORE the handoff copy instead
 *    of after — the second assertion failed (staging deleted before the
 *    copy could read it would be actively broken, but the structural check
 *    here catches the ordering regression directly).
 * 3. Removed `cleanupStagedPlaintext()` from the `catch` block — the third
 *    assertion failed.
 * 4. Removed the internal cleanup-on-failure inside
 *    `copyToSystemTemporaryDirectory`'s permission-fixup `catch` — the
 *    fourth assertion failed.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const SWIFT_PATH = join(import.meta.dir, '..', 'BeebeebFileProvider', 'FileProviderExtension.swift')

/** Return the substring from `openBraceIndex` (which must point at a `{`)
 * through its matching `}`, by simple depth counting. Neither function this
 * file inspects has string literals or comments containing braces between
 * its outer braces and the blocks this file extracts, so plain counting is
 * safe here. */
function extractBalancedBlock(source: string, openBraceIndex: number): string {
  if (source[openBraceIndex] !== '{') {
    throw new Error(`expected '{' at index ${openBraceIndex}, got ${JSON.stringify(source[openBraceIndex])}`)
  }
  let depth = 0
  for (let i = openBraceIndex; i < source.length; i++) {
    if (source[i] === '{') depth++
    else if (source[i] === '}') {
      depth--
      if (depth === 0) return source.slice(openBraceIndex, i + 1)
    }
  }
  throw new Error(`unbalanced braces starting at index ${openBraceIndex}`)
}

function extractFunctionBody(source: string, signature: string): string {
  const sigIndex = source.indexOf(signature)
  if (sigIndex === -1) throw new Error(`could not find ${JSON.stringify(signature)} in ${SWIFT_PATH}`)
  const braceIndex = source.indexOf('{', sigIndex)
  if (braceIndex === -1) throw new Error(`could not find opening brace for ${JSON.stringify(signature)}`)
  return extractBalancedBlock(source, braceIndex)
}

describe('FileProviderExtension.fetchContents (task 1670 round 4)', () => {
  const source = readFileSync(SWIFT_PATH, 'utf8')
  const fnBody = extractFunctionBody(source, 'func fetchContents(')

  const doIndex = fnBody.indexOf('do {')
  if (doIndex === -1) throw new Error('fetchContents must contain a do/catch block hydrating the file')
  const doBlock = extractBalancedBlock(fnBody, fnBody.indexOf('{', doIndex))

  const catchIndex = fnBody.indexOf('catch {', doIndex + doBlock.length)
  if (catchIndex === -1) throw new Error('fetchContents must contain a catch block after the do block')
  const catchBlock = extractBalancedBlock(fnBody, fnBody.indexOf('{', catchIndex))

  test('success path copies to the system handoff directory and hands off THAT URL, never destinationURL', () => {
    expect(doBlock).toContain('copyToSystemTemporaryDirectory(stagedAt: destinationURL)')
    // The literal call `completionHandler(destinationURL, ...)` (round 3's
    // shape) must be gone — completionHandler must be called with the
    // handoff copy's URL instead.
    expect(doBlock).not.toContain('completionHandler(destinationURL')
    expect(doBlock).toContain('completionHandler(handoffURL')
  })

  test('success path deletes OUR staging copy only AFTER the handoff copy succeeds', () => {
    const copyIndex = doBlock.indexOf('copyToSystemTemporaryDirectory(stagedAt: destinationURL)')
    const cleanupIndex = doBlock.indexOf('cleanupStagedPlaintext()')
    const completionIndex = doBlock.indexOf('completionHandler(handoffURL')
    expect(copyIndex).toBeGreaterThan(-1)
    expect(cleanupIndex).toBeGreaterThan(-1)
    expect(completionIndex).toBeGreaterThan(-1)
    expect(cleanupIndex).toBeGreaterThan(copyIndex)
    // Cleanup of OUR OWN staging copy is safe either before or after handing
    // the (DIFFERENT, already-copied) handoff URL to completionHandler — the
    // load-bearing order is copy-before-cleanup, asserted above. This just
    // pins that cleanup did not move back before the copy.
    expect(completionIndex).toBeGreaterThan(copyIndex)
  })

  test('every failure path still deletes the staged plaintext file', () => {
    // Nothing was handed to the system on a failure path — the delete here
    // must stay, or a failed hydration leaks its staged plaintext forever.
    expect(catchBlock).toContain('cleanupStagedPlaintext()')
  })
})

describe('FileProviderExtension.copyToSystemTemporaryDirectory (task 1670 round 4)', () => {
  const source = readFileSync(SWIFT_PATH, 'utf8')
  const fnBody = extractFunctionBody(source, 'private func copyToSystemTemporaryDirectory(')

  test('resolves the domain-scoped NSFileProviderManager and its system temporaryDirectoryURL()', () => {
    expect(fnBody).toContain('NSFileProviderManager(for: domain)')
    expect(fnBody).toContain('manager.temporaryDirectoryURL()')
  })

  test('copies the staged file rather than deleting/moving the original', () => {
    expect(fnBody).toContain('FileManager.default.copyItem(at: sourceURL, to: destinationURL)')
  })

  test('forces owner-only permissions on the copy', () => {
    expect(fnBody).toContain('0o600')
  })

  test('cleans up its own partial copy before rethrowing on a post-copy failure', () => {
    // The permission-fixup catch block is the only place a partial copy can
    // exist when this function fails (the copy itself succeeded, a later
    // step didn't) — it must remove that copy before propagating the error.
    const setAttributesIndex = fnBody.indexOf('setAttributes')
    const catchAfterIndex = fnBody.indexOf('catch {', setAttributesIndex)
    expect(setAttributesIndex).toBeGreaterThan(-1)
    expect(catchAfterIndex).toBeGreaterThan(-1)
    const catchBlock = extractBalancedBlock(fnBody, fnBody.indexOf('{', catchAfterIndex))
    expect(catchBlock).toContain('removeItem(at: destinationURL)')
  })
})
