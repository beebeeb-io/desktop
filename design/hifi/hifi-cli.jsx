// hifi-cli.jsx — Wave 8: bb CLI reference terminal

function CliTerm({ title = 'isa@mac · ~/vault', width = 820, height = 560, children }) {
  return (
    <div style={{
      width, height, borderRadius: 10, overflow: 'hidden',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.4), 0 0 0 1px #0b0b0c',
      background: '#0c0c0d', color: '#e9e6dd', fontFamily: 'var(--font-mono)', fontSize: 12.5,
      display: 'flex', flexDirection: 'column',
    }}>
      <div style={{
        height: 34, background: '#1a1a1c', borderBottom: '1px solid #2a2a2d',
        display: 'flex', alignItems: 'center', padding: '0 12px', position: 'relative',
      }}>
        <div style={{ display: 'flex', gap: 7 }}>
          {['#ff5f57', '#febc2e', '#28c840'].map((c, i) => (
            <div key={i} style={{ width: 12, height: 12, borderRadius: '50%', background: c }} />
          ))}
        </div>
        <div style={{
          position: 'absolute', left: 0, right: 0, textAlign: 'center',
          fontSize: 11.5, color: '#9a958a', pointerEvents: 'none',
        }}>{title}</div>
      </div>
      <div style={{ flex: 1, overflow: 'auto', padding: '14px 18px', lineHeight: 1.55 }}>
        {children}
      </div>
    </div>
  );
}

const cliColors = {
  prompt: '#f5b800',
  cmd: '#e9e6dd',
  dim: '#6a655b',
  comment: '#7d8a6a',
  amber: '#f5b800',
  green: '#8fc18b',
  red: '#e07a6a',
  cyan: '#7fb8d1',
  path: '#d0c89a',
};

function Line({ children, c }) { return <div style={{ color: c || cliColors.cmd }}>{children}</div>; }
function Prompt({ cmd }) {
  return (
    <div style={{ marginTop: 4 }}>
      <span style={{ color: cliColors.prompt }}>bb</span>
      <span style={{ color: cliColors.dim }}> ~/vault</span>
      <span style={{ color: cliColors.amber }}> ❯</span>{' '}
      <span style={{ color: cliColors.cmd }}>{cmd}</span>
    </div>
  );
}
function Comment({ children }) { return <span style={{ color: cliColors.comment }}>{children}</span>; }

// ─── Help screen ─────────────────────────────────────
function CliHelp() {
  return (
    <CliTerm title="isa@mac · bb --help">
      <Prompt cmd="bb --help" />
      <Line c={cliColors.amber}>
        <span style={{ fontWeight: 700 }}>BEEBEEB</span> <span style={{ color: cliColors.dim }}>— end-to-end encrypted vault, from the terminal</span>
      </Line>
      <Line c={cliColors.dim}>v0.4.1 · {'{paris, frankfurt, stockholm}'} · <span style={{ color: cliColors.green }}>e2ee</span></Line>
      <Line> </Line>
      <Line c={cliColors.amber}>USAGE</Line>
      <Line c={cliColors.dim}>  bb &lt;command&gt; [options]</Line>
      <Line> </Line>
      <Line c={cliColors.amber}>COMMANDS</Line>
      {[
        ['ls', '[path]', 'list files (decrypts names locally)'],
        ['cp', '&lt;src&gt; &lt;dest&gt;', 'copy · encrypts on the fly if dest is a vault path'],
        ['mv', '&lt;src&gt; &lt;dest&gt;', 'move / rename'],
        ['rm', '&lt;path&gt;', 'move to trash (recoverable 30d)'],
        ['push', '[path]', 'upload a file or folder to your vault'],
        ['pull', '&lt;path&gt;', 'download to stdout or a local path'],
        ['share', '&lt;path&gt;', 'create an encrypted link (pass, expiry, opens)'],
        ['mount', '&lt;dir&gt;', 'FUSE-mount the vault as a folder'],
        ['rotate', '', 'rotate your master key · re-encrypts metadata'],
        ['whoami', '', 'session · device · region · quota'],
        ['devices', '', 'list devices holding a copy of your key'],
      ].map(([c, a, d], i) => (
        <Line key={i}>
          <span style={{ color: cliColors.green, display: 'inline-block', width: 80 }}>  {c}</span>
          <span style={{ color: cliColors.path, display: 'inline-block', width: 180 }} dangerouslySetInnerHTML={{ __html: a }} />
          <span style={{ color: cliColors.dim }}>{d}</span>
        </Line>
      ))}
      <Line> </Line>
      <Line c={cliColors.amber}>GLOBAL FLAGS</Line>
      <Line><span style={{ color: cliColors.cyan, width: 80, display: 'inline-block' }}>  --region</span><span style={{ color: cliColors.dim }}>pin operation to a data center (paris|frankfurt|stockholm)</span></Line>
      <Line><span style={{ color: cliColors.cyan, width: 80, display: 'inline-block' }}>  --quiet</span><span style={{ color: cliColors.dim }}>no progress · machine-readable output</span></Line>
      <Line><span style={{ color: cliColors.cyan, width: 80, display: 'inline-block' }}>  --json</span><span style={{ color: cliColors.dim }}>emit structured JSON</span></Line>
      <Line> </Line>
      <Line><Comment># docs · beebeeb.io/cli · key fingerprints · beebeeb.io/fingerprints</Comment></Line>
      <Prompt cmd={<span style={{ background: cliColors.amber, color: '#000', padding: '0 3px' }}>&nbsp;</span>} />
    </CliTerm>
  );
}

// ─── Upload session ─────────────────────────────────────
function CliUpload() {
  return (
    <CliTerm title="isa@mac · bb push investigations/">
      <Prompt cmd="bb whoami" />
      <Line><span style={{ color: cliColors.dim }}>user   </span> isa.marchetti <span style={{ color: cliColors.dim }}>&lt;isa@example.com&gt;</span></Line>
      <Line><span style={{ color: cliColors.dim }}>device </span> MacBook Pro · <span style={{ color: cliColors.path }}>MBP-8F2A</span></Line>
      <Line><span style={{ color: cliColors.dim }}>region </span> <span style={{ color: cliColors.amber }}>frankfurt</span> · 14 ms</Line>
      <Line><span style={{ color: cliColors.dim }}>quota  </span> <span style={{ color: cliColors.green }}>23.4 GB</span> / 200 GB <span style={{ color: cliColors.dim }}>(11.7%)</span></Line>
      <Line> </Line>
      <Prompt cmd={<span>bb push <span style={{ color: cliColors.path }}>./investigations/ledger-gap/</span> <span style={{ color: cliColors.cyan }}>--region frankfurt</span></span>} />
      <Line c={cliColors.dim}>  resolved → investigations/ledger-gap/  (23 files, 142 MB)</Line>
      <Line c={cliColors.dim}>  encrypting with vault key <span style={{ color: cliColors.path }}>bbvk_4e9a…c7d1</span> (XChaCha20-Poly1305)</Line>
      <Line> </Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span> story-draft.md               <span style={{ color: cliColors.dim }}>14.2 kB</span>   <span style={{ color: cliColors.amber }}>100%</span>   32 ms</Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span> source-A-transcript.pdf      <span style={{ color: cliColors.dim }}>2.4 MB</span>    <span style={{ color: cliColors.amber }}>100%</span>   180 ms</Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span> exhibit-B-contract.pdf       <span style={{ color: cliColors.dim }}>1.1 MB</span>    <span style={{ color: cliColors.amber }}>100%</span>   88 ms</Line>
      <Line>  <span style={{ color: cliColors.cyan }}>↑</span> interview-03.m4a             <span style={{ color: cliColors.dim }}>48 MB</span>     <span style={{ color: cliColors.amber }}>━━━━━━━━━━━━━━━━━━━━━━━━─── 72%</span></Line>
      <Line c={cliColors.dim}>    chunk 231/320 · 14 MB/s · <span style={{ color: cliColors.green }}>e2ee</span> · eta 2s</Line>
      <Line>  <span style={{ color: cliColors.dim }}>·</span> footnotes-draft.md           <span style={{ color: cliColors.dim }}>queued</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>·</span> fact-check-notes.pdf         <span style={{ color: cliColors.dim }}>queued</span></Line>
      <Line> </Line>
      <Line c={cliColors.dim}>  18 of 23 complete · 124 MB / 142 MB</Line>
      <Prompt cmd={<span style={{ background: cliColors.amber, color: '#000', padding: '0 3px' }}>&nbsp;</span>} />
    </CliTerm>
  );
}

// ─── Share session ─────────────────────────────────────
function CliShare() {
  return (
    <CliTerm title="isa@mac · bb share">
      <Prompt cmd={<span>bb ls <span style={{ color: cliColors.path }}>investigations/ledger-gap/</span></span>} />
      <Line>  <span style={{ color: cliColors.path }}>story-draft.md</span>              <span style={{ color: cliColors.dim }}>14.2 kB   4 min ago</span></Line>
      <Line>  <span style={{ color: cliColors.path }}>source-A-transcript.pdf</span>     <span style={{ color: cliColors.dim }}>2.4 MB    2h ago</span></Line>
      <Line>  <span style={{ color: cliColors.path }}>exhibit-B-contract.pdf</span>      <span style={{ color: cliColors.dim }}>1.1 MB    2h ago</span></Line>
      <Line>  <span style={{ color: cliColors.path }}>leak-packet.pdf</span>             <span style={{ color: cliColors.dim }}>1.1 MB    6h ago</span></Line>
      <Line> </Line>
      <Prompt cmd={<span>bb share <span style={{ color: cliColors.path }}>leak-packet.pdf</span> <span style={{ color: cliColors.cyan }}>--expires 48h --max-opens 1 --passphrase -</span></span>} />
      <Line c={cliColors.amber}>  ? Passphrase (12+ chars, mixed): <span style={{ background: 'rgba(245,184,0,0.15)', color: cliColors.amber, padding: '0 4px' }}>••••••••••••••••</span></Line>
      <Line c={cliColors.dim}>    Entropy: 87 bits · <span style={{ color: cliColors.green }}>strong</span></Line>
      <Line> </Line>
      <Line c={cliColors.dim}>  wrapping chunk keys with Argon2id(passphrase)</Line>
      <Line c={cliColors.dim}>  depositing ciphertext → frankfurt/shares/48e1…</Line>
      <Line> </Line>
      <Line>  <span style={{ color: cliColors.green }}>✓ Link created</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>url       </span> https://bee.beebeeb.io/s/<span style={{ color: cliColors.amber }}>hx7-mk2-qrf-wnd-ztj</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>expires   </span> 2026-04-24T09:41Z <span style={{ color: cliColors.dim }}>(in 47h 59m)</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>max-opens </span> 1</Line>
      <Line>  <span style={{ color: cliColors.dim }}>region    </span> <span style={{ color: cliColors.amber }}>frankfurt</span> (pinned · will not replicate)</Line>
      <Line>  <span style={{ color: cliColors.dim }}>receipt   </span> <span style={{ color: cliColors.path }}>bbr_914c3a2f…e91</span></Line>
      <Line> </Line>
      <Line><Comment># send the passphrase by a different channel · we will never see it</Comment></Line>
      <Line><Comment># revoke anytime:  bb revoke bbr_914c3a2f</Comment></Line>
      <Prompt cmd={<span style={{ background: cliColors.amber, color: '#000', padding: '0 3px' }}>&nbsp;</span>} />
    </CliTerm>
  );
}

// ─── Key rotation ─────────────────────────────────────
function CliRotate() {
  return (
    <CliTerm title="isa@mac · bb rotate">
      <Prompt cmd={<span>bb rotate <span style={{ color: cliColors.cyan }}>--confirm</span></span>} />
      <Line> </Line>
      <Line c={cliColors.amber}>  ▲ This will rotate your master vault key.</Line>
      <Line c={cliColors.dim}>    Re-wraps file keys · re-encrypts metadata blobs.</Line>
      <Line c={cliColors.dim}>    All 5 devices must re-authenticate.</Line>
      <Line c={cliColors.dim}>    File ciphertexts are unchanged (keys rewrapped only).</Line>
      <Line> </Line>
      <Line c={cliColors.amber}>  ? Current passphrase: <span style={{ background: 'rgba(245,184,0,0.15)', color: cliColors.amber, padding: '0 4px' }}>••••••••••••••••••</span></Line>
      <Line c={cliColors.amber}>  ? 2FA (passkey touch): <span style={{ color: cliColors.green }}>✓ verified</span></Line>
      <Line> </Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span> generated new key        <span style={{ color: cliColors.path }}>bbvk_a18f…2d4c</span></Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span> rewrapped file keys      <span style={{ color: cliColors.dim }}>2,834 entries · 1.4s</span></Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span> re-encrypted index       <span style={{ color: cliColors.dim }}>(names, paths, tags)</span></Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span> revoked previous key     <span style={{ color: cliColors.path }}>bbvk_4e9a…c7d1</span></Line>
      <Line>  <span style={{ color: cliColors.amber }}>⌁</span> pending device re-auth   <span style={{ color: cliColors.dim }}>iPhone 15 Pro, Pixel 8, Win-Desktop, bb-cli-home</span></Line>
      <Line> </Line>
      <Line c={cliColors.dim}>  Your recovery phrase is unchanged. Nothing to re-export.</Line>
      <Line> </Line>
      <Line c={cliColors.green}>  Done in 2.1s. Old ciphertexts remain readable only through your wrap history.</Line>
      <Line><Comment># audit this:  bb log --type key.rotate --last 1h</Comment></Line>
      <Prompt cmd={<span style={{ background: cliColors.amber, color: '#000', padding: '0 3px' }}>&nbsp;</span>} />
    </CliTerm>
  );
}

Object.assign(window, { CliHelp, CliUpload, CliShare, CliRotate, CliRclone, CliArchive, CliSDK, CliCron });

// ─── rclone integration ─────────────────────────────────────
function CliRclone() {
  return (
    <CliTerm title="devops@box · rclone config · bb backend">
      <Prompt cmd="rclone config create beebeeb bb token ~/.config/bb/token region frankfurt" />
      <Line c={cliColors.green}>  ✓ remote "beebeeb:" registered</Line>
      <Line c={cliColors.dim}>    backend: bb (bundled · beebeeb.io/rclone)</Line>
      <Line c={cliColors.dim}>    e2e layer: XChaCha20-Poly1305 · keys stay on this box</Line>
      <Line> </Line>
      <Prompt cmd={<span>rclone sync <span style={{ color: cliColors.path }}>/srv/backups/pg/</span> <span style={{ color: cliColors.path }}>beebeeb:archive/pg/2026-04/</span> <span style={{ color: cliColors.cyan }}>--progress --transfers 16 --bb-encrypt</span></span>} />
      <Line c={cliColors.dim}>  Transferring:</Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span>  dump-2026-04-20.sql.zst   <span style={{ color: cliColors.dim }}>842 MB</span>  <span style={{ color: cliColors.amber }}>━━━━━━━━━━━━ 100%</span>  12 s</Line>
      <Line>  <span style={{ color: cliColors.green }}>✓</span>  dump-2026-04-21.sql.zst   <span style={{ color: cliColors.dim }}>847 MB</span>  <span style={{ color: cliColors.amber }}>━━━━━━━━━━━━ 100%</span>  13 s</Line>
      <Line>  <span style={{ color: cliColors.cyan }}>↑</span>  dump-2026-04-22.sql.zst   <span style={{ color: cliColors.dim }}>853 MB</span>  <span style={{ color: cliColors.amber }}>━━━━━━━━━───── 64%</span></Line>
      <Line c={cliColors.dim}>    throughput 188 MB/s · 4 workers · e2ee overhead 0.6%</Line>
      <Line> </Line>
      <Line c={cliColors.dim}>  Elapsed: 38.2s · Transferred: 2.1 GB / 3.3 GB · ETA 22s</Line>
      <Line> </Line>
      <Line><Comment># daily cron pattern:</Comment></Line>
      <Line c={cliColors.dim}>  <span style={{ color: cliColors.path }}>0 3 * * *</span> rclone sync /srv/backups beebeeb:archive --bb-encrypt --delete-after</Line>
      <Line> </Line>
      <Line><Comment># same backend works with restic, borg, rsync-over-fuse (bb mount)</Comment></Line>
      <Prompt cmd={<span style={{ background: cliColors.amber, color: '#000', padding: '0 3px' }}>&nbsp;</span>} />
    </CliTerm>
  );
}

// ─── Cold-storage / archivist ─────────────────────────────────────
function CliArchive() {
  return (
    <CliTerm title="archivist@box · bb archive">
      <Prompt cmd={<span>bb archive create <span style={{ color: cliColors.cyan }}>--name "Court-Case-2024-0412"</span> <span style={{ color: cliColors.cyan }}>--retention 10y --class cold --witness legal@firm</span></span>} />
      <Line c={cliColors.dim}>  scanning → /evidence/2024-0412/  (11,284 files · 47.2 GB)</Line>
      <Line c={cliColors.dim}>  hashing  → SHA-256 + BLAKE3 (dual) · 2m 14s</Line>
      <Line c={cliColors.dim}>  sealing  → PQC envelope (Kyber-1024 + XChaCha20)</Line>
      <Line> </Line>
      <Line>  <span style={{ color: cliColors.green }}>✓ Archive sealed</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>id         </span> <span style={{ color: cliColors.path }}>bba_0412-ct-2024-b7e…</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>manifest   </span> <span style={{ color: cliColors.path }}>bba_0412.manifest.json</span> <span style={{ color: cliColors.dim }}>(signed · co-signed by legal@firm)</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>retention  </span> <span style={{ color: cliColors.amber }}>legal hold · until 2034-04-22</span> <span style={{ color: cliColors.dim }}>(immutable)</span></Line>
      <Line>  <span style={{ color: cliColors.dim }}>class      </span> cold · replicated · Frankfurt + Paris</Line>
      <Line>  <span style={{ color: cliColors.dim }}>quantum    </span> <span style={{ color: cliColors.green }}>hybrid PQ+classical</span></Line>
      <Line> </Line>
      <Prompt cmd={<span>bb archive verify <span style={{ color: cliColors.path }}>bba_0412-ct-2024-b7e</span></span>} />
      <Line c={cliColors.dim}>  checking manifest signatures · 2/2 co-signers verified</Line>
      <Line c={cliColors.dim}>  sampling 1,024 random chunks · verifying BLAKE3 → ciphertext</Line>
      <Line c={cliColors.dim}>  checking replica drift · Frankfurt ↔ Paris</Line>
      <Line>  <span style={{ color: cliColors.green }}>✓ Integrity OK</span> <span style={{ color: cliColors.dim }}>· 0 chunks drifted · 0 missing</span></Line>
      <Line>  <span style={{ color: cliColors.green }}>✓ Chain of custody intact</span> <span style={{ color: cliColors.dim }}>· next scheduled verify: 2026-07-22</span></Line>
      <Line> </Line>
      <Line><Comment># export a tamper-evident bundle for an auditor:</Comment></Line>
      <Line c={cliColors.dim}>  $ bb archive export bba_0412 --with-manifest --with-proofs ./bundle.tar</Line>
      <Prompt cmd={<span style={{ background: cliColors.amber, color: '#000', padding: '0 3px' }}>&nbsp;</span>} />
    </CliTerm>
  );
}

// ─── Node SDK (developer) ─────────────────────────────────────
function CliSDK() {
  return (
    <CliTerm title="dev@mac · editor · upload.ts">
      <Line c={cliColors.dim}>// upload.ts — end-to-end encrypted upload with receipts</Line>
      <Line c={cliColors.dim}>// npm i @beebeeb/sdk   ·   keys stay in process memory</Line>
      <Line> </Line>
      <Line><span style={{ color: cliColors.cyan }}>import</span> <span>&#123; Beebeeb &#125;</span> <span style={{ color: cliColors.cyan }}>from</span> <span style={{ color: cliColors.green }}>'@beebeeb/sdk'</span><span style={{ color: cliColors.dim }}>;</span></Line>
      <Line><span style={{ color: cliColors.cyan }}>import</span> <span>fs</span> <span style={{ color: cliColors.cyan }}>from</span> <span style={{ color: cliColors.green }}>'node:fs'</span><span style={{ color: cliColors.dim }}>;</span></Line>
      <Line> </Line>
      <Line><span style={{ color: cliColors.cyan }}>const</span> <span style={{ color: cliColors.amber }}>bb</span> <span style={{ color: cliColors.dim }}>=</span> <span style={{ color: cliColors.cyan }}>new</span> <span style={{ color: cliColors.path }}>Beebeeb</span>(&#123;</Line>
      <Line c={cliColors.dim}>  region<span>:</span> <span style={{ color: cliColors.green }}>'frankfurt'</span>,</Line>
      <Line c={cliColors.dim}>  auth<span>:</span>   <span>&#123; tokenEnv: <span style={{ color: cliColors.green }}>'BB_TOKEN'</span> &#125;</span>,</Line>
      <Line c={cliColors.dim}>  key<span>:</span>    <span>&#123; passphraseEnv: <span style={{ color: cliColors.green }}>'BB_KEY_PASS'</span> &#125;</span><span style={{ color: cliColors.dim }}>,</span></Line>
      <Line><span style={{ color: cliColors.dim }}>&#125;);</span></Line>
      <Line> </Line>
      <Line><span style={{ color: cliColors.cyan }}>const</span> <span style={{ color: cliColors.amber }}>receipt</span> <span style={{ color: cliColors.dim }}>=</span> <span style={{ color: cliColors.cyan }}>await</span> bb.<span style={{ color: cliColors.path }}>push</span>(&#123;</Line>
      <Line c={cliColors.dim}>  source<span>:</span> fs.<span style={{ color: cliColors.path }}>createReadStream</span>(<span style={{ color: cliColors.green }}>'./invoice-2411.pdf'</span>),</Line>
      <Line c={cliColors.dim}>  path<span>:</span>   <span style={{ color: cliColors.green }}>'billing/2026/04/invoice-2411.pdf'</span>,</Line>
      <Line c={cliColors.dim}>  meta<span>:</span>   <span>&#123; customer: <span style={{ color: cliColors.green }}>'acme'</span>, amount: <span style={{ color: cliColors.amber }}>4800</span> &#125;</span>,</Line>
      <Line c={cliColors.dim}>  onProgress<span>:</span> (p) <span style={{ color: cliColors.cyan }}>=&gt;</span> <span style={{ color: cliColors.path }}>log</span>(<span style={{ color: cliColors.green }}>`${'{'}p.pct.toFixed(0){'}'}%`</span>),</Line>
      <Line><span style={{ color: cliColors.dim }}>&#125;);</span></Line>
      <Line> </Line>
      <Line c={cliColors.path}>console</Line>
      <Line><span style={{ color: cliColors.dim }}>// &#123;</span></Line>
      <Line><span style={{ color: cliColors.dim }}>//   id:   </span> <span style={{ color: cliColors.green }}>'bbf_9a2c3d…e4f1'</span><span style={{ color: cliColors.dim }}>,</span></Line>
      <Line><span style={{ color: cliColors.dim }}>//   sha256: </span><span style={{ color: cliColors.green }}>'8b4e…a1f'</span><span style={{ color: cliColors.dim }}>,</span></Line>
      <Line><span style={{ color: cliColors.dim }}>//   region: </span><span style={{ color: cliColors.amber }}>'frankfurt'</span><span style={{ color: cliColors.dim }}>,</span></Line>
      <Line><span style={{ color: cliColors.dim }}>//   sealed: </span><span style={{ color: cliColors.green }}>'2026-04-22T14:31:09Z'</span><span style={{ color: cliColors.dim }}> &#125;</span></Line>
      <Line> </Line>
      <Line><Comment># also: bb.share(), bb.pull(), bb.list(), bb.verify(), bb.rotate()</Comment></Line>
      <Line><Comment># webhook HMAC signing + typed event payloads in docs</Comment></Line>
    </CliTerm>
  );
}

// ─── Engineer cron pipeline ─────────────────────────────────────
function CliCron() {
  return (
    <CliTerm title="ops@server · /etc/cron.d/bb-backup">
      <Line c={cliColors.dim}># /etc/cron.d/bb-backup  — nightly e2ee backup of Postgres + uploads</Line>
      <Line c={cliColors.dim}># Beebeeb stays EU-pinned · Frankfurt · key in HashiCorp Vault</Line>
      <Line> </Line>
      <Line><span style={{ color: cliColors.path }}>SHELL</span><span style={{ color: cliColors.dim }}>=/bin/bash</span></Line>
      <Line><span style={{ color: cliColors.path }}>BB_TOKEN</span><span style={{ color: cliColors.dim }}>=</span><span style={{ color: cliColors.green }}>{'{{ vault:bb/token }}'}</span></Line>
      <Line><span style={{ color: cliColors.path }}>BB_KEY_PASS</span><span style={{ color: cliColors.dim }}>=</span><span style={{ color: cliColors.green }}>{'{{ vault:bb/kpass }}'}</span></Line>
      <Line> </Line>
      <Line c={cliColors.dim}># minute hour dom mon dow  user  command</Line>
      <Line><span style={{ color: cliColors.amber }}>0  3  *  *  *</span>  root  <span style={{ color: cliColors.path }}>/usr/local/bin/bb-night.sh</span> <span style={{ color: cliColors.dim }}>&gt;&gt; /var/log/bb.log 2&gt;&amp;1</span></Line>
      <Line><span style={{ color: cliColors.amber }}>0  4  *  *  0</span>  root  <span style={{ color: cliColors.path }}>bb archive verify --all --random 2048</span></Line>
      <Line><span style={{ color: cliColors.amber }}>30 2  1  *  *</span>  root  <span style={{ color: cliColors.path }}>bb rotate --quiet --json</span> | logger -t bb-rotate</Line>
      <Line> </Line>
      <Line c={cliColors.dim}># /usr/local/bin/bb-night.sh</Line>
      <Line c={cliColors.dim}>---</Line>
      <Line><span style={{ color: cliColors.amber }}>set</span> <span style={{ color: cliColors.dim }}>-euo pipefail</span></Line>
      <Line><span style={{ color: cliColors.dim }}>TS=$(date -u +%F)</span></Line>
      <Line> </Line>
      <Line c={cliColors.dim}># stream-encrypt straight into your vault — no unencrypted temp files</Line>
      <Line><span style={{ color: cliColors.path }}>pg_dump</span> --format=c prod \</Line>
      <Line c={cliColors.dim}>  | zstd -T0 -10 \</Line>
      <Line c={cliColors.dim}>  | <span style={{ color: cliColors.path }}>bb push</span> --stdin --path <span style={{ color: cliColors.green }}>"db/${'{'}TS{'}'}/prod.sql.zst"</span> --json \</Line>
      <Line c={cliColors.dim}>  | <span style={{ color: cliColors.path }}>jq</span> -r <span style={{ color: cliColors.green }}>'.id'</span> &gt;&gt; /var/log/bb/receipts.txt</Line>
      <Line> </Line>
      <Line><span style={{ color: cliColors.path }}>bb push</span> /srv/uploads/ <span style={{ color: cliColors.cyan }}>--mirror --delete-after 14d --quiet</span></Line>
      <Line><span style={{ color: cliColors.path }}>bb prune</span> db/ <span style={{ color: cliColors.cyan }}>--keep 30d --keep-weekly 12</span></Line>
      <Line> </Line>
      <Line><Comment># all writes are immutable until retention expires · receipts piped to SIEM</Comment></Line>
      <Prompt cmd={<span style={{ background: cliColors.amber, color: '#000', padding: '0 3px' }}>&nbsp;</span>} />
    </CliTerm>
  );
}
