# The Office helper's test documents

Every file here is ours: written by a script in this repository, with no third-party content. The latency harness
keeps its own files in `latency/`; they are not listed here.

| File | Origin | Pages | sha256 |
|---|---|---|---|
| `letter-1.docx` | `make_fixtures.py` (this folder, Python standard library only) | 1, US Letter, 612 x 792 pt | `dbb146f747d011076de17719b192ab7298d79306e34c238545cbf1f498990c01` |
| `system-font.docx` | `make_fixtures.py` (this folder) | 1, A4 | `b86be86b76493768fcffdcd93a1574a8aa2b2fafbd40b06f1f38b3aee78ab42e` |
| `spike.docx` | copied byte for byte from `spikes/2477-office-editor-throwaway/fixtures/spike.docx` (commit `92281ffd`, written by that folder's `make-fixtures.py`) | 3, A4 | `2eb05e616c92c2609930d3a19132492a1889a37c6430c59ae36ea158e256bd1f` |
| `remote.docx` | copied byte for byte from `spikes/2477-office-editor-throwaway/fixtures/remote.docx` (commit `92281ffd`, same script) | 1, A4, a picture **linked** to `http://127.0.0.1:47477/remote.png` | `4569ff876dc0968186663b0414233fa16f03d4eac2ca03e6b9998a849f629f89` |

- **`letter-1.docx` is the page that shows the one page-pixel rule.** Every spike fixture is A4, where single and
  double precision agree. A Letter page at 4/3 (100 % on a 96 DPI screen) is 816 x 1056 px in single precision, the
  Office engine's rule, and 817 x 1057 in double. `tests/serve.rs` checks the shared vector
  (`crates/beebeeb-office-protocol/tests/page_pixels_vectors.txt`) against `letter-1.docx` (its `letter-*` rows) and
  `spike.docx` (its `a4-*` rows).
- **`system-font.docx`** has a heading in "Segoe Script", a handwriting face Windows installs and the Office engine does
  not bundle (its bundled fallback is a sans), and one line in the default face. In a page rendered on Windows, a
  handwritten heading can only have come from the system's font folder. No test here asserts it; the integration
  lane looks at it.
- **`remote.docx`** must render with "Some content isn't shown." and fetch nothing: a listener on port 47477 counts
  the connections (`a_remote_picture_says_missing_content_and_fetches_nothing`).

`make_fixtures.py` is deterministic (fixed timestamps and order): run it again (Python 3.15.0 wrote the files above)
and compare the sums. A different zlib may compress differently; then the sums change and the documents do not.
