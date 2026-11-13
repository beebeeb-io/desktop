# M4: the final map audit (night note)

Slice `06-the-final-map-audit-counts-every-action`, 2026-11-13. Measurement only; no product code
changed. The full report is `evidence/desktop-v2/m4-final-audit/report.md` (and the same file under
`.claude/tasks/verification-evidence/desktop-v2/m4-final-audit/`).

## Counts

Full run, 2026-11-13T04:39:30+01:00 → 04:50:38+01:00, exit 0:

```
actions=846 ok=270 missing-control=96 no-effect=26 wrong-target=71 command-unsupported=196 not-automatable=156 source-unreached=31 frame-renderer-only=0
```

846 actions from 329 frames (Mail Backup excluded); 731 rows opened live; 73 rows from 27 frames
carry a declared gap; 31 rows from 14 frames are `source-unreached` (their state recipe failed before
the action was tried, listed in `map-audit.md`).

Plant run, exit 0:

```
actions=12 ok=6 missing-control=1 no-effect=0 wrong-target=0 command-unsupported=3 not-automatable=2 source-unreached=0 frame-renderer-only=0
planted DW-0401#8: missing-control (entry state, state ready)
```

The plant was red first: before the dev-host repair it read `planted DW-0401#8: source-unreached
(entry state, state failed)` and exited 1.

## The two ruled corrections

The corrected board copy is `/tmp/opencode/frames-m4.json`; the workspace `frames.json` was not
edited. The design correction stays a design task.

- DW-0602#1 `to: ["DW-0601"]` (was DW-0610). Outcome: `wrong-target`, target score 0.48. The live
  screen is DW-0601 plus the Undo toast; the only token DW-0601 adds over the DW-0602 start is the
  place heading (`Sunday, 12 October · Lisbon`), which fork 2 says no command draws yet. The
  correction moved the score from 0.37 to 0.48; it does not clear the 0.6 bar. This is the one
  acceptance row that does not flip; the honest result is recorded, not hidden.
- DW-1304#0 `to: ["DW-1303"]` (was DW-1301). Outcome: `no-effect`, not `wrong-target`; the row also
  carries its declared gap (other devices' changes are in no command on this PC).

## Instrument repair (dev-only)

The auth gate moved to Rust (task 2445) and fails closed when the host cannot answer
`auth_gate_state`; the dev host answered no such read, so the first plant run opened no state (10 of
12 rows `source-unreached`). Repaired in dev-only code, never bundled:

- `src/v2/dev/reads.ts`: `auth_gate_state: 'ready'` in `BOARD_READS`.
- `src/v2/dev/states-windows.ts`: `'signed_out'` in `OUT`, `'locked'` for DW-0009, `'no_plan'` for
  DW-0014.

## Reproduce

```bash
cd /home/guus/code/bb-worktrees/desktop-v2-night-2
bun run dev -- --port 5225 --strictPort &
bun -e 'const f="/home/guus/code/beebeeb.io/design/desktop-v2/frames.json";const d=JSON.parse(await Bun.file(f).text());const fr=d.Windows.frames;fr.find(x=>x.id==="DW-0602").notes.Actions[1].to=["DW-0601"];fr.find(x=>x.id==="DW-1304").notes.Actions[0].to=["DW-1303"];await Bun.write("/tmp/opencode/frames-m4.json",JSON.stringify(d))'
BB_WORKSPACE=/home/guus/code/beebeeb.io bun scripts/v2-map-audit/audit.mjs --base http://localhost:5225 --only DW-0401 --plant --out /tmp/opencode/m4-plant
BB_WORKSPACE=/home/guus/code/beebeeb.io bun scripts/v2-map-audit/audit.mjs --base http://localhost:5225 --board /tmp/opencode/frames-m4.json --workers 4 --out /tmp/opencode/m4-audit
```

Checks: `bun test tests/v2MapAuditCompare.test.ts` → 6 pass / 0 fail; `bunx tsc --noEmit` → exit 0.
