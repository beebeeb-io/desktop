# big-60.docx

- Origin: `spikes/2477-office-editor-throwaway/fixtures/big-30-pages.docx`, commit `92281ffd`, copied byte for byte.
- Made by: the same folder's `make-fixtures.py` (Python standard library only).
- Size: 2,745 bytes. sha256: `ad9e90bba3d4a50cdc0aa4072db559e3bdc5789ca5cf3aa53a83a8972882926e`.
- A4; page count depends on the fonts of the machine: the test prints it. The spike called it "30 pages"; it lays
  out as 60 with the build VMs' fonts.
- Used by `tests/latency.rs` only.

# grid-2000.xlsx

- Made by: `../make_fixtures.py` (this repository, Python standard library only; Python 3.15.0 wrote it).
- Size: 246,012 bytes. sha256: `8f3cd40381e555079a087b12784de750a5f35de7c6c8410fefda7f4a0f4615b0`.
- One sheet "Grid": a bold header row (frozen), then 1,999 rows of 20 columns: a label, then amounts (`#,##0.00`),
  shares (`0.0%`) and counts (general) in turn; one column chart over B2:B13, anchored over V2:AC17.
- Used by `tests/latency.rs` only (`latency_cells_p95_within_16ms`: visible ranges of 60 x 20).
