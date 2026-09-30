/**
 * Task 1670 round 3 (lead review of round 2, PR #75).
 *
 * Apple's own `fetchContents` docs (developer.apple.com/tutorials/data/
 * documentation/fileprovider/nsfileproviderreplicatedextension/fetchcontents
 * (for:version:request:completionhandler:).json) say "After you call the
 * completion handler, the system takes complete control over the local
 * copy" and "...so that the system can clone it..." — but nowhere say that
 * clone happens SYNCHRONOUSLY, inside the `completionHandler` call itself.
 * Round 2 of this task deleted the staged plaintext file unconditionally
 * right after a SUCCESSFUL `completionHandler` call. If `fileproviderd`
 * actually clones asynchronously (which the docs neither confirm nor rule
 * out), that delete races the clone and can reopen the exact "Couldn't
 * communicate with a helper application" bug this task exists to fix.
 *
 * `BeebeebFileProvider` (the File Provider extension target) has no XCTest
 * target in this repo (checked: no `.xcodeproj`/`.xctestplan`, no existing
 * Swift test file under `tests/`), so — same pattern as this repo's own
 * `noHardcodedThemeColors.test.ts` and `noAdHocErrorSurface.test.ts` guards
 * — this reads the real Swift source text and pins the shape structurally:
 *
 * - the SUCCESS path (the `do` block, up to the first thrown error) must
 *   still hand `destinationURL` to `completionHandler`, but must NEVER also
 *   call `cleanupStagedPlaintext()` — the system may still be cloning that
 *   file asynchronously after this call returns.
 * - every FAILURE path (the `catch` block) must still call
 *   `cleanupStagedPlaintext()` — nothing was ever handed to the system
 *   there, so it is both safe and necessary to clean up immediately.
 *
 * Mutation check (2026-09-30, reverted after confirming RED both times):
 * temporarily re-added `cleanupStagedPlaintext()` right after the success
 * `completionHandler(destinationURL, ...)` call in `fetchContents`'s `do`
 * block (reintroducing round 2's exact bug) — the first assertion below
 * failed, naming the do-block text. Separately, temporarily removed
 * `cleanupStagedPlaintext()` from the `catch` block — the second assertion
 * failed. Both reverted; see the task file's Notes for the pasted output.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const SWIFT_PATH = join(import.meta.dir, '..', 'BeebeebFileProvider', 'FileProviderExtension.swift')

/** Return the substring from `openBraceIndex` (which must point at a `{`)
 * through its matching `}`, by simple depth counting. `fetchContents` has no
 * string literals or comments containing braces between its outer function
 * braces and the do/catch block, so this plain counting is safe here. */
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

describe('FileProviderExtension.fetchContents (task 1670 round 3)', () => {
  const source = readFileSync(SWIFT_PATH, 'utf8')
  const fnBody = extractFunctionBody(source, 'func fetchContents(')

  const doIndex = fnBody.indexOf('do {')
  if (doIndex === -1) throw new Error('fetchContents must contain a do/catch block hydrating the file')
  const doBlock = extractBalancedBlock(fnBody, fnBody.indexOf('{', doIndex))

  const catchIndex = fnBody.indexOf('catch {', doIndex + doBlock.length)
  if (catchIndex === -1) throw new Error('fetchContents must contain a catch block after the do block')
  const catchBlock = extractBalancedBlock(fnBody, fnBody.indexOf('{', catchIndex))

  test('success path hands the file to completionHandler', () => {
    expect(doBlock).toContain('completionHandler(destinationURL')
  })

  test('success path does NOT delete the staged plaintext file', () => {
    // This is the round-2 bug this round reverts: deleting immediately after
    // a SUCCESSFUL completionHandler call races an undocumented-timing clone.
    expect(doBlock).not.toContain('cleanupStagedPlaintext()')
  })

  test('every failure path still deletes the staged plaintext file', () => {
    // Nothing was handed to the system on a failure path — the delete here
    // must stay, or a failed hydration leaks its staged plaintext forever.
    expect(catchBlock).toContain('cleanupStagedPlaintext()')
  })
})
