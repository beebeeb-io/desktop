// hifi-actions.jsx — Wave 3: move/copy, trash, version history, search results

// ─── Move / Copy modal ─────────────────────────────────────
function HiMoveModal() {
  return (
    <div style={{
      width: 520, background: 'var(--paper)', borderRadius: 'var(--r-3)',
      border: '1px solid var(--line-2)', boxShadow: 'var(--shadow-3)', overflow: 'hidden',
    }}>
      <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <Ico name="folder" size={14} color="var(--amber-deep)" />
        <div>
          <div className="t-h3" style={{ fontSize: 14 }}>Move 3 items</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Destination will re-encrypt with the folder's key</div>
        </div>
        <div style={{ marginLeft: 'auto', width: 28, height: 28, borderRadius: 6, background: 'var(--paper-2)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--ink-3)', transform: 'rotate(45deg)' }}>
          <Ico name="plus" size={13} color="currentColor" />
        </div>
      </div>

      {/* Selected items strip */}
      <div style={{ padding: '10px 18px', background: 'var(--paper-2)', borderBottom: '1px solid var(--line)', display: 'flex', gap: 6, flexWrap: 'wrap' }}>
        {[
          ['file', 'protest-0147.cr3'],
          ['file', 'source-B-transcript.pdf'],
          ['folder', 'field-recordings/'],
        ].map(([ic, n], i) => (
          <BBChip key={i} style={{ fontSize: 10.5 }}>
            <Ico name={ic} size={10} />
            {n}
          </BBChip>
        ))}
      </div>

      {/* Breadcrumb */}
      <div style={{ padding: '10px 18px', display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, color: 'var(--ink-2)', borderBottom: '1px solid var(--line)' }}>
        <span style={{ color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>My files</span>
        <Ico name="chevRight" size={11} color="var(--ink-4)" />
        <span style={{ color: 'var(--amber-deep)', fontWeight: 500, cursor: 'pointer' }}>investigations</span>
        <Ico name="chevRight" size={11} color="var(--ink-4)" />
        <span style={{ fontWeight: 600 }}>ledger-gap</span>
      </div>

      {/* Search */}
      <div style={{ padding: '10px 14px', borderBottom: '1px solid var(--line)' }}>
        <div className="bb-input">
          <Ico name="search" size={12} color="var(--ink-3)" />
          <input placeholder="Search or type a new folder name…" />
        </div>
      </div>

      {/* Folder list */}
      <div style={{ maxHeight: 240, overflow: 'auto' }}>
        {[
          ['sources', 47, false],
          ['drafts', 12, true],
          ['published-2024', 208, false],
          ['fixer-contacts', 6, false],
          ['interviews-sep2025', 18, false],
          ['_archive', 1240, false],
        ].map(([name, count, active], i) => (
          <div key={i} style={{
            padding: '10px 18px', display: 'flex', alignItems: 'center', gap: 10,
            background: active ? 'var(--amber-bg)' : 'transparent',
            borderLeft: active ? '3px solid var(--amber-deep)' : '3px solid transparent',
            cursor: 'pointer',
          }}>
            <Ico name="folder" size={14} color={active ? 'var(--amber-deep)' : 'var(--ink-3)'} />
            <span style={{ fontSize: 13, fontWeight: active ? 600 : 400, flex: 1 }}>{name}</span>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{count}</span>
            {active && <Ico name="check" size={13} color="var(--amber-deep)" />}
          </div>
        ))}
      </div>

      {/* Footer */}
      <div style={{ padding: '12px 18px', borderTop: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11.5, color: 'var(--ink-3)' }}>
          <Ico name="plus" size={11} /> New folder
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          <BBBtn size="sm">Copy here</BBBtn>
          <BBBtn size="sm" variant="amber">Move here</BBBtn>
        </div>
      </div>
    </div>
  );
}

// ─── Trash ─────────────────────────────────────
function HiTrash() {
  const items = [
    { ic: 'image', name: 'test-shot-001.jpg', where: 'field-recordings/', deleted: '2h ago', expires: '28 days', size: '4.2 MB' },
    { ic: 'file', name: 'draft-v1-OLD.md', where: 'drafts/', deleted: 'yesterday', expires: '29 days', size: '14 KB' },
    { ic: 'folder', name: 'test-exports/', where: '/', deleted: '3 days ago', expires: '27 days', size: '120 MB · 48 items' },
    { ic: 'image', name: 'screenshot-2025-09-14-09.png', where: 'inbox/', deleted: '8 days ago', expires: '22 days', size: '2.1 MB' },
    { ic: 'file', name: 'notes-scratch.txt', where: 'inbox/', deleted: '14 days ago', expires: '16 days', size: '6 KB' },
    { ic: 'image', name: 'duplicate-IMG_0047.heic', where: 'photos/', deleted: '22 days ago', expires: '8 days', size: '3.8 MB', urgent: true },
  ];
  return (
    <div style={{ width: 920 }}>
      <div className="bb-card" style={{ padding: 0 }}>
        <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
          <Ico name="trash" size={14} />
          <div>
            <div className="t-h3" style={{ fontSize: 14 }}>Trash</div>
            <div className="t-micro" style={{ color: 'var(--ink-3)' }}>Items auto-shredded 30 days after deletion · your vault key is destroyed last</div>
          </div>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <BBBtn size="sm">Restore all</BBBtn>
            <BBBtn size="sm" variant="danger">Empty trash</BBBtn>
          </div>
        </div>

        {/* Table header */}
        <div style={{
          display: 'grid', gridTemplateColumns: '24px 1.4fr 1fr 110px 110px 110px 60px',
          gap: 14, padding: '10px 18px', borderBottom: '1px solid var(--line)',
          background: 'var(--paper-2)',
        }}>
          <div />
          <span className="t-label">Name</span>
          <span className="t-label">Was in</span>
          <span className="t-label">Deleted</span>
          <span className="t-label">Shreds in</span>
          <span className="t-label">Size</span>
          <span />
        </div>
        {items.map((it, i, arr) => (
          <div key={i} style={{
            display: 'grid', gridTemplateColumns: '24px 1.4fr 1fr 110px 110px 110px 60px',
            gap: 14, padding: '10px 18px', alignItems: 'center',
            borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
          }}>
            <BBCheck />
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
              <Ico name={it.ic} size={14} color={it.ic === 'folder' ? 'var(--amber-deep)' : 'var(--ink-3)'} />
              <span style={{ fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: 'var(--ink-2)' }}>{it.name}</span>
            </div>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{it.where}</span>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{it.deleted}</span>
            <span className="t-mono" style={{
              fontSize: 11, fontWeight: it.urgent ? 600 : 400,
              color: it.urgent ? 'var(--red)' : 'var(--ink-3)',
            }}>{it.expires}</span>
            <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>{it.size}</span>
            <div style={{ display: 'flex', gap: 4, justifyContent: 'flex-end' }}>
              <span style={{ cursor: 'pointer', color: 'var(--amber-deep)', fontSize: 11.5, fontWeight: 500 }}>Restore</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Version history ─────────────────────────────────────
function HiVersionHistory() {
  const versions = [
    { v: 'v47', who: 'you · MacBook', when: '4 min ago', bytes: '18.4 KB', delta: '+2 paragraphs', current: true },
    { v: 'v46', who: 'you · iPhone', when: '2h ago', bytes: '17.9 KB', delta: '+47 words · corrections' },
    { v: 'v45', who: 'you · MacBook', when: 'yesterday · 23:14', bytes: '17.6 KB', delta: '−1 paragraph · restructure' },
    { v: 'v44', who: 'you · MacBook', when: 'yesterday · 18:02', bytes: '16.8 KB', delta: '+1,200 words · new section II' },
    { v: 'v43', who: 'you · MacBook', when: '2 days ago', bytes: '15.1 KB', delta: '+84 words' },
    { v: 'v42', who: 'you · iPhone', when: '3 days ago', bytes: '14.9 KB', delta: 'minor edits', milestone: 'Shared with editor' },
    { v: 'v41', who: 'you · MacBook', when: '4 days ago', bytes: '14.7 KB', delta: '+2,100 words · draft II complete' },
    { v: 'v40', who: 'you · MacBook', when: '6 days ago', bytes: '12.4 KB', delta: 'draft II begun' },
  ];
  return (
    <div style={{
      width: 860, background: 'var(--paper)', borderRadius: 'var(--r-3)',
      border: '1px solid var(--line-2)', boxShadow: 'var(--shadow-3)', overflow: 'hidden',
      display: 'grid', gridTemplateColumns: '300px 1fr',
    }}>
      {/* Timeline */}
      <div style={{ borderRight: '1px solid var(--line)', background: 'var(--paper-2)' }}>
        <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--line)' }}>
          <div className="t-h3" style={{ fontSize: 14 }}>Version history</div>
          <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 2 }}>story-draft.md · 47 versions</div>
        </div>
        <div style={{ padding: '4px 0', position: 'relative' }}>
          {/* vertical line */}
          <div style={{ position: 'absolute', left: 28, top: 14, bottom: 14, width: 1, background: 'var(--line-2)' }} />
          {versions.map((v, i) => (
            <div key={i} style={{
              padding: '10px 16px 10px 40px',
              background: v.current ? 'var(--paper)' : 'transparent',
              borderLeft: v.current ? '3px solid var(--amber-deep)' : '3px solid transparent',
              position: 'relative',
              cursor: 'pointer',
            }}>
              <div style={{
                position: 'absolute', left: 23, top: 14,
                width: 11, height: 11, borderRadius: '50%',
                background: v.current ? 'var(--amber)' : 'var(--paper)',
                border: '2px solid', borderColor: v.current ? 'var(--amber-deep)' : 'var(--line-2)',
              }} />
              <div style={{ display: 'flex', alignItems: 'baseline', gap: 6 }}>
                <span className="t-mono" style={{ fontSize: 11.5, fontWeight: 600 }}>{v.v}</span>
                <span className="t-micro" style={{ color: 'var(--ink-3)' }}>· {v.when}</span>
              </div>
              <div className="t-micro" style={{ color: 'var(--ink-3)', marginTop: 1 }}>{v.who}</div>
              <div style={{ fontSize: 11.5, color: 'var(--ink-2)', marginTop: 3 }}>{v.delta}</div>
              {v.milestone && <BBChip variant="amber" style={{ fontSize: 9, marginTop: 4 }}>{v.milestone}</BBChip>}
            </div>
          ))}
        </div>
      </div>

      {/* Diff pane */}
      <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
        <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 10 }}>
          <span className="t-mono" style={{ fontSize: 11.5, color: 'var(--ink-3)' }}>v46 → v47</span>
          <span className="t-micro" style={{ color: 'var(--ink-4)' }}>· 4 min ago</span>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
            <BBBtn size="sm" variant="ghost">Download v47</BBBtn>
            <BBBtn size="sm" variant="amber">Restore this version</BBBtn>
          </div>
        </div>
        <div className="bb-scroll" style={{ flex: 1, padding: '16px 18px', fontFamily: 'var(--font-mono)', fontSize: 11.5, lineHeight: 1.7, minHeight: 0 }}>
          <div style={{ color: 'var(--ink-3)', marginBottom: 12 }}>
            <div>@@ story-draft.md · section II. The call</div>
            <div className="t-micro" style={{ marginTop: 2 }}>+34 lines · −8 lines · 18.4 KB total</div>
          </div>
          {[
            { t: ' ', c: '> Source B was a mid-level compliance officer. They remember the' },
            { t: ' ', c: '> call because it was unusual in two respects: it came from a' },
            { t: ' ', c: '> number outside the organisation, and the person on the line' },
            { t: '-', c: '> asked them something strange.' },
            { t: '+', c: '> asked them to do something they had never been asked to do in' },
            { t: '+', c: '> fourteen years on the job.' },
            { t: ' ', c: '' },
            { t: '+', c: "> The caller didn't identify themselves, but they knew things" },
            { t: '+', c: '> that, in B\'s recollection, only three other people should have' },
            { t: '+', c: '> known. "I thought it was a test," B told me later. "I almost' },
            { t: '+', c: '> hung up."' },
          ].map((ln, i) => (
            <div key={i} style={{
              padding: '1px 10px',
              background: ln.t === '+' ? 'oklch(0.96 0.04 155 / 0.5)' : ln.t === '-' ? 'oklch(0.97 0.02 25 / 0.5)' : 'transparent',
              borderLeft: '2px solid',
              borderColor: ln.t === '+' ? 'oklch(0.72 0.16 155)' : ln.t === '-' ? 'var(--red)' : 'transparent',
              color: ln.t === '-' ? 'oklch(0.45 0.08 25)' : ln.t === '+' ? 'oklch(0.3 0.1 155)' : 'var(--ink-2)',
            }}>
              <span style={{ color: 'var(--ink-4)', marginRight: 8 }}>{ln.t}</span>{ln.c}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── Search results ─────────────────────────────────────
function HiSearchResults() {
  const results = [
    {
      ic: 'file', name: 'story-draft.md', path: 'investigations/ledger-gap/',
      match: '…the silence was <mark>deliberate</mark>. According to seven people familiar with the matter…',
      size: '18 KB', modified: '4 min ago',
    },
    {
      ic: 'file', name: 'source-B-interview.md', path: 'investigations/ledger-gap/sources/',
      match: '…they noticed the discrepancy was <mark>deliberate</mark>, not a systems error…',
      size: '47 KB', modified: '3 days ago',
    },
    {
      ic: 'folder', name: 'deliberate-omissions/', path: 'investigations/2024/',
      match: null,
      size: '14 items · 1.2 GB', modified: '8 months ago',
    },
    {
      ic: 'file', name: 'editorial-notes-2024.pdf', path: 'work/published-2024/',
      match: '…headline was changed from "intentional" to "<mark>deliberate</mark>" per Anika\'s note…',
      size: '124 KB', modified: '1 year ago',
    },
  ];
  return (
    <div style={{ width: 820, background: 'var(--paper)', borderRadius: 'var(--r-3)', border: '1px solid var(--line)', overflow: 'hidden', boxShadow: 'var(--shadow-2)' }}>
      {/* Search bar */}
      <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
        <div className="bb-input" style={{ padding: '10px 12px' }}>
          <Ico name="search" size={14} color="var(--amber-deep)" />
          <input defaultValue="deliberate" style={{ fontSize: 14 }} />
          <BBKbd>Esc</BBKbd>
        </div>
        {/* Filter chips */}
        <div style={{ display: 'flex', gap: 6, marginTop: 10, flexWrap: 'wrap' }}>
          <BBChip variant="filled" style={{ fontSize: 10.5 }}>All · 4</BBChip>
          <BBChip style={{ fontSize: 10.5 }}>Docs · 3</BBChip>
          <BBChip style={{ fontSize: 10.5 }}>Folders · 1</BBChip>
          <BBChip style={{ fontSize: 10.5 }}>Images · 0</BBChip>
          <BBChip style={{ fontSize: 10.5 }}>This year</BBChip>
          <BBChip style={{ fontSize: 10.5, color: 'var(--amber-deep)', borderColor: 'oklch(0.86 0.07 90)', background: 'var(--amber-bg)' }}>
            <Ico name="shield" size={10} /> Encrypted search · device-only
          </BBChip>
        </div>
      </div>

      {/* Results */}
      <div>
        {results.map((r, i, arr) => (
          <div key={i} style={{
            padding: '14px 18px',
            borderBottom: i < arr.length - 1 ? '1px solid var(--line)' : 'none',
            cursor: 'pointer',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <Ico name={r.ic} size={14} color={r.ic === 'folder' ? 'var(--amber-deep)' : 'var(--ink-2)'} />
              <span style={{ fontSize: 13.5, fontWeight: 500 }}>{r.name}</span>
              <span className="t-mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>· {r.path}</span>
              <span className="t-mono" style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--ink-3)' }}>{r.size} · {r.modified}</span>
            </div>
            {r.match && (
              <div
                style={{
                  fontSize: 12.5, color: 'var(--ink-3)', marginTop: 6,
                  paddingLeft: 24, lineHeight: 1.5,
                }}
                dangerouslySetInnerHTML={{ __html: r.match.replace(/<mark>/g, '<span style="background: var(--amber-bg); color: var(--ink); padding: 0 2px; border-radius: 2px; font-weight: 500;">').replace(/<\/mark>/g, '</span>') }}
              />
            )}
          </div>
        ))}
      </div>

      <div style={{ padding: '10px 18px', background: 'var(--paper-2)', borderTop: '1px solid var(--line)', display: 'flex', alignItems: 'center', gap: 8, fontSize: 11, color: 'var(--ink-3)' }}>
        <BBKbd>↑↓</BBKbd> navigate · <BBKbd>↵</BBKbd> open · <BBKbd>⌘↵</BBKbd> new tab
        <span style={{ marginLeft: 'auto' }}>Search ran on your device · content never sent to server</span>
      </div>
    </div>
  );
}

Object.assign(window, { HiMoveModal, HiTrash, HiVersionHistory, HiSearchResults });
