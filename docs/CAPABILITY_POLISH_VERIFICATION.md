# Task 1611 polish — 2026-09-29

Shared capability explanations now use the existing Card, NavIcon, typography
tokens and desktop button classes. This tree has no BBButton export. Resolution
failure remains inline because it blocks loading (1248/1255), with separate Retry
and Open web app actions. No capability logic or native behavior changed.

Local evidence lives in workspace `.claude/tasks/verification-evidence/1611/polish/`:

- Rendering regression proof before implementation: 5 pass / 3 fail, each for a
  missing shared notice in Sync, unsupported routes or Advanced.
- Retry styling mutation: 8 pass / 1 fail, specifically the missing styled Retry
  button; source restored before the final checks.
- `bun test`: 107 pass / 0 fail, 259 assertions.
- `bunx tsc --noEmit -p . --listFiles`: exit 0, 136 files, 0 diagnostics.
- `bun run lint`: exit 0. ESLint JSON: 38 files, 0 errors, 0 warnings.
- Browser fixture bundle: 31 modules. Runner syntax check: exit 0.
- Graphify AST refresh: 158 files, 2484 nodes, 5208 edges. No doc semantic pass.

Chromium could not launch: `setsockopt: Operation not permitted`. **0 new browser
cases, 0 new screenshots**; no visual or native pass claimed. Earlier attempts
using the cached Playwright entry hit a missing dependency, then a missing default
browser binary; the explicit system Chromium attempt establishes the sandbox gap.

Lead rerun, using the existing local Playwright installation (no dependency change):

```sh
bun build tests/fixtures/capability-render.tsx --target browser --format iife --outfile "$evidence/capability-render.js"
node tests/render-capability-fixtures.mjs "$repo" "$evidence"
```

Set `PLAYWRIGHT_MODULE` to the installed Playwright module and `CHROME_PATH` if
using system Chromium. Expect 12 cases: original settings/onboarding/routes,
styled card/body/button checks, successful Retry recovery, web opener actions,
and three compact dark-mode states with keyboard focus and no horizontal overflow.
The workspace NATIVE.md points to this versioned runner. Original lead screenshots
are preserved. Native/Rust gates are not rerun for this presentation-only change.
