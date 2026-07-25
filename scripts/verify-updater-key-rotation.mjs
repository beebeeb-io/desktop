#!/usr/bin/env node

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash, createPublicKey, verify } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(__dirname, '..')
const testNewKeyPassword = 'task-1240 throwaway passphrase for updater key rotation verifier'
const trustedCommentPrefix = 'trusted comment: '
const ed25519SpkiPrefix = Buffer.from('302a300506032b6570032100', 'hex')

// This proves the updater trust-chain sequencing with throwaway keys. It must
// never require or load the real old production updater private key.

function fail(message) {
  throw new Error(message)
}

function commandForTauriCli() {
  const configured = process.env.TAURI_CLI
  const candidates = [
    configured,
    path.join(repoRoot, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'),
  ].filter(Boolean)

  const tauriCli = candidates.find((candidate) => fs.existsSync(candidate))
  if (!tauriCli) {
    fail(
      'Could not find the Tauri CLI. Run `bun install` first or set TAURI_CLI=/path/to/node_modules/@tauri-apps/cli/tauri.js.',
    )
  }

  return tauriCli.endsWith('.js')
    ? { command: process.execPath, prefixArgs: [tauriCli] }
    : { command: tauriCli, prefixArgs: [] }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    input: options.input ?? '',
    env: { ...process.env, ...options.env },
  })

  if (!options.allowFailure && result.status !== 0) {
    fail(
      [
        `Command failed: ${command} ${args.join(' ')}`,
        `exit: ${result.status}`,
        result.stdout.trim(),
        result.stderr.trim(),
      ]
        .filter(Boolean)
        .join('\n'),
    )
  }

  return result
}

function runTauri(args, options = {}) {
  const cli = commandForTauriCli()
  return run(cli.command, [...cli.prefixArgs, ...args], options)
}

function generateKeypair(privateKeyPath, password) {
  const args = ['signer', 'generate', '--ci', '-w', privateKeyPath, '-p', password]
  runTauri(args)
  return fs.readFileSync(`${privateKeyPath}.pub`, 'utf8').trim()
}

function signPayload(privateKeyPath, password, payloadPath) {
  const args = ['signer', 'sign', '-f', privateKeyPath, '-p', password, payloadPath]

  runTauri(args)
  const signaturePath = `${payloadPath}.sig`
  return fs.readFileSync(signaturePath, 'utf8').trim()
}

function decodeTauriBase64Text(value) {
  return Buffer.from(value, 'base64').toString('utf8')
}

function decodeMinisignBase64Line(value, expectedLength, label) {
  const decoded = Buffer.from(value, 'base64')
  assert.equal(decoded.length, expectedLength, `${label} should decode to ${expectedLength} bytes`)
  return decoded
}

function parseTauriPubkey(tauriPubkey) {
  const lines = decodeTauriBase64Text(tauriPubkey).trimEnd().split(/\r?\n/)
  assert.ok(lines.length >= 2, 'minisign public key should have at least two lines')

  const keyRecord = decodeMinisignBase64Line(lines[1], 42, 'minisign public key')
  const algorithm = keyRecord.subarray(0, 2).toString('ascii')
  assert.match(algorithm, /^E[dD]$/, 'minisign public key should use an Ed25519 algorithm marker')

  return {
    keyId: keyRecord.subarray(2, 10),
    publicKey: createPublicKey({
      key: Buffer.concat([ed25519SpkiPrefix, keyRecord.subarray(10, 42)]),
      format: 'der',
      type: 'spki',
    }),
  }
}

function parseTauriSignature(tauriSignature) {
  const lines = decodeTauriBase64Text(tauriSignature).trimEnd().split(/\r?\n/)
  assert.ok(lines.length >= 4, 'minisign signature should have four lines')
  assert.ok(lines[2].startsWith(trustedCommentPrefix), 'minisign signature should include a trusted comment')

  const signatureRecord = decodeMinisignBase64Line(lines[1], 74, 'minisign signature')
  const algorithm = signatureRecord.subarray(0, 2).toString('ascii')
  assert.match(algorithm, /^E[dD]$/, 'minisign signature should use an Ed25519 algorithm marker')

  return {
    algorithm,
    keyId: signatureRecord.subarray(2, 10),
    signature: signatureRecord.subarray(10, 74),
    trustedComment: lines[2].slice(trustedCommentPrefix.length),
    globalSignature: decodeMinisignBase64Line(lines[3], 64, 'minisign global signature'),
  }
}

function signedPayloadBytes(payload, signatureAlgorithm) {
  if (signatureAlgorithm === 'Ed') {
    return payload
  }
  if (signatureAlgorithm === 'ED') {
    return createHash('blake2b512').update(payload).digest()
  }
  fail(`Unsupported minisign signature algorithm: ${signatureAlgorithm}`)
}

function verifyTauriUpdaterSignature(tauriPubkey, tauriSignature, payload) {
  const publicKey = parseTauriPubkey(tauriPubkey)
  const signature = parseTauriSignature(tauriSignature)

  if (!publicKey.keyId.equals(signature.keyId)) {
    return false
  }

  const fileSignatureOk = verify(
    null,
    signedPayloadBytes(payload, signature.algorithm),
    publicKey.publicKey,
    signature.signature,
  )
  if (!fileSignatureOk) {
    return false
  }

  const trustedCommentSignatureOk = verify(
    null,
    Buffer.concat([signature.signature, Buffer.from(signature.trustedComment, 'utf8')]),
    publicKey.publicKey,
    signature.globalSignature,
  )
  return trustedCommentSignatureOk
}

function main() {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'beebeeb-updater-key-rotation-'))

  try {
    const oldKeyPath = path.join(tmpDir, 'old.key')
    const newKeyPath = path.join(tmpDir, 'new.key')
    const oldPubkey = generateKeypair(oldKeyPath, '')
    const newPubkey = generateKeypair(newKeyPath, testNewKeyPassword)

    const transitionPackage = path.join(tmpDir, 'release-n-transition.bin')
    const nextPackage = path.join(tmpDir, 'release-n-plus-1.bin')
    fs.writeFileSync(transitionPackage, `release N package with baked new pubkey:\n${newPubkey}\n`)
    fs.writeFileSync(nextPackage, 'release N+1 package signed after installed clients trust the new pubkey\n')

    const transitionSignature = signPayload(oldKeyPath, '', transitionPackage)
    const nextSignature = signPayload(newKeyPath, testNewKeyPassword, nextPackage)

    assert.equal(
      verifyTauriUpdaterSignature(oldPubkey, transitionSignature, fs.readFileSync(transitionPackage)),
      true,
      'old install should accept transition release signed with the old key',
    )
    assert.equal(
      verifyTauriUpdaterSignature(newPubkey, nextSignature, fs.readFileSync(nextPackage)),
      true,
      'new install should accept next release signed with the new key',
    )
    assert.equal(
      verifyTauriUpdaterSignature(newPubkey, transitionSignature, fs.readFileSync(transitionPackage)),
      false,
      'new install should reject a package signed with the old key',
    )
    assert.equal(
      verifyTauriUpdaterSignature(oldPubkey, nextSignature, fs.readFileSync(nextPackage)),
      false,
      'old install should reject a package signed with the new key',
    )

    const withoutPassword = runTauri(
      ['signer', 'sign', '-f', newKeyPath, nextPackage],
      { allowFailure: true, input: '' },
    )
    assert.notEqual(withoutPassword.status, 0, 'new protected key must reject signing without its passphrase')

    console.log(
      [
        'updater key rotation verifier passed:',
        '- old install accepted transition release signed with the old key',
        '- new install accepted N+1 release signed with the new key',
        '- new install rejected the old-key transition signature',
        '- old install rejected the new-key N+1 signature',
        '- passphrase-protected new key rejected signing without its passphrase',
      ].join('\n'),
    )
  } finally {
    if (process.env.KEEP_UPDATER_KEY_ROTATION_TMP === '1') {
      console.log(`kept temporary verifier directory: ${tmpDir}`)
    } else {
      fs.rmSync(tmpDir, { recursive: true, force: true })
    }
  }
}

main()
