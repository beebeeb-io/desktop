# M4: the final map audit

Generated 2026-11-13T04:50:38+01:00 from the worktree `desktop-v2-night-2` (branch
`night/2026-11-12-06-the-final-map-audit-counts-every-action`), dev server on `http://localhost:5225`.

## The run

```
BB_WORKSPACE=/home/guus/code/beebeeb.io bun scripts/v2-map-audit/audit.mjs \
  --base http://localhost:5225 --board /tmp/opencode/frames-m4.json --workers 4 \
  --out /tmp/opencode/m4-audit
```

Started 2026-11-13T04:39:30+01:00, finished 2026-11-13T04:50:38+01:00, exit 0.

```
actions=846 ok=270 missing-control=96 no-effect=26 wrong-target=71 command-unsupported=196 not-automatable=156 source-unreached=31 frame-renderer-only=0
```

| Outcome | Count |
|---|---:|
| ok | 270 |
| missing-control | 96 |
| no-effect | 26 |
| wrong-target | 71 |
| command-unsupported | 196 |
| not-automatable | 156 |
| source-unreached | 31 |
| frame-renderer-only | 0 |
| **total** | **846** |

846 actions from 329 frames (Mail Backup excluded). Rows opened live: 731. Not opened (every way to
trigger the action is not automatable, or the frame is not built): 115. Opened live but the live app
cannot draw the frame's whole state (a declared gap in the recipe): 73 rows from 27 frames. Opened
live with less than 0.6 of the frame's signature on screen: 21 rows.

`source-unreached` is not `ok`: 31 rows from 14 frames whose state recipe failed before the action
could be tried (DW-A23, DW-A24, DW-A25, DW-A26, DW-A27, DW-A28, DW-0748, DW-0749, DW-0749b,
DW-0511, DW-0512, DW-0512b, DW-0513, DW-0514). They are listed in `map-audit.md` with the step that
found no control.

## The plant: the check proves it can fail

```
BB_WORKSPACE=/home/guus/code/beebeeb.io bun scripts/v2-map-audit/audit.mjs \
  --base http://localhost:5225 --only DW-0401 --plant --out /tmp/opencode/m4-plant
```

Red, before the dev-host repair below (2026-11-13T04:29:47+01:00 → 04:31:19+01:00, exit 1):

```
actions=12 ok=0 missing-control=0 no-effect=0 wrong-target=0 command-unsupported=0 not-automatable=2 source-unreached=10 frame-renderer-only=0
planted DW-0401#8: source-unreached (entry state, state failed)
```

Green, after the repair (2026-11-13T04:37:50+01:00 → 04:39:10+01:00, exit 0):

```
actions=12 ok=6 missing-control=1 no-effect=0 wrong-target=0 command-unsupported=3 not-automatable=2 source-unreached=0 frame-renderer-only=0
planted DW-0401#8: missing-control (entry state, state ready)
```

The planted row is `DW-0401#8`, the action whose control name was replaced with a label that exists
nowhere. It came back `missing-control` from a live state (`entry state`, `state ready`), and the run
exited 0 only because of that.

## The two ruled board-data corrections

The corrected copy is `/tmp/opencode/frames-m4.json`, made from the workspace
`design/desktop-v2/frames.json` without editing it:

```
bun -e 'const f="/home/guus/code/beebeeb.io/design/desktop-v2/frames.json";const d=JSON.parse(await Bun.file(f).text());const fr=d.Windows.frames;fr.find(x=>x.id==="DW-0602").notes.Actions[1].to=["DW-0601"];fr.find(x=>x.id==="DW-1304").notes.Actions[0].to=["DW-1303"];await Bun.write("/tmp/opencode/frames-m4.json",JSON.stringify(d))'
```

| Row | Board `to` | Corrected `to` | Outcome | Detail |
|---|---|---|---|---|
| DW-0602#1 | DW-0610 | DW-0601 | wrong-target | an unrecognised screen (target score 0.48): b:Undo Ctrl+Z / st:Moved to Trash Undo Ctrl+Z |
| DW-1304#0 | DW-1301 | DW-1303 | no-effect | (declared gap: other devices' changes are in no command on this PC) |

DW-1304#0 is not in the wrong-target list. DW-0602#1 still is, and the correction is not the cause:
the live screen after "Move to Trash" is DW-0601 plus the Undo toast, and the only token DW-0601
adds over the DW-0602 start is the day heading's place (`Sunday, 12 October · Lisbon`). Fork 2 of
`v2-m3d-board-contradictions-and-forks-ruled.md` says no command draws that place yet, so the target
scores 0.48 (0.5 × 0.97 cover + 0.5 × 0 distinct) against the audit's 0.6 bar. The ruled `to`
correction moved the score from 0.37 (M3d, target DW-0610) to 0.48; it does not clear the bar until
the place is drawn or the audit treats DW-0601/DW-0602 as one screen with another selection. Both
are outside this slice (product code / `scripts/v2-map-audit/*`).

## The instrument repair (dev-only, never bundled)

The first plant run opened no state: the auth gate moved to Rust (task 2445) and fails closed when
the host cannot answer `auth_gate_state`, while the dev host (`src/v2/dev/reads.ts`) answered no such
read. Every row was `source-unreached` (10 of 12 in the plant run). The repair is dev-only and never
bundled:

- `src/v2/dev/reads.ts`: `auth_gate_state: 'ready'` in `BOARD_READS` (a browser state is signed in
  unless its recipe says otherwise).
- `src/v2/dev/states-windows.ts`: `auth_gate_state: 'signed_out'` in `OUT` (the sign-in states),
  `'locked'` for DW-0009, `'no_plan'` for DW-0014.

No product module changed. `tests/v2NoFixturesInProduction.test.ts` still guards that production
imports neither `src/v2/source/fixtures` nor `src/v2/dev`.

## Checks

| Check | Result |
|---|---|
| `bun test tests/v2MapAuditCompare.test.ts` | 6 pass / 0 fail (22 expect calls) |
| `bunx tsc --noEmit` | exit 0 (2026-11-13T04:55:01+01:00 → 04:55:08+01:00) |
| plant run | exit 0, planted row `missing-control` |
| full run | exit 0, 846 actions |

`map-audit.json` (359,195 bytes, under 2 MB) is beside this report.
