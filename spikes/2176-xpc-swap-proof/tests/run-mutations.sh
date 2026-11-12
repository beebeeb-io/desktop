#!/usr/bin/env bash
# spike(2176): prove the SwapCore harness can go red. Each mutation edits a THROWAWAY copy of
# SwapCore.swift (the source file is never touched), compiles it with the harness and runs it.
# A mutation passes this script only if the harness FAILS, and only on the tests named for it.
#   tests/run-mutations.sh [work-dir]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$HERE/../BeebeebSwapProbe/SwapCore.swift"
WORK="${1:-$(mktemp -d)}"
mkdir -p "$WORK"
red=0
total=0

# name | perl substitution applied to the copy | a test that must FAIL
mutations=(
  "symlink-inside-skipped|s/case S_IFLNK:\n(\s+)return SwapOutcome\(\.symlinkRefused, \.copy\)/case S_IFLNK:\n\1continue/|FAIL a symlink inside the staged bundle"
  "equal-counts-as-newer|s/(if a != b \{ return a > b \}\n\s+\}\n\s+)return false/\1return true/|FAIL equal version (rule 1)"
  "sibling-kept-on-refusal|s/_ = removeTree\(siblingPath\)/_ = siblingPath/|FAIL an unsigned bundle"
  "container-string-prefix|s/Array\(staged\.prefix\(rootCount\)\) == policy\.containerRoot/stagedPath.hasPrefix(PathCheck.join(policy.containerRoot))/|FAIL staged path sharing the container's string prefix (Data-evil)"
)

for m in "${mutations[@]}"; do
  IFS='|' read -r name expr must <<<"$m"
  total=$((total + 1))
  dir="$WORK/$name"
  rm -rf "$dir"
  mkdir -p "$dir"
  cp "$SRC" "$dir/SwapCore.swift"
  perl -0pi -e "$expr" "$dir/SwapCore.swift"
  if cmp -s "$SRC" "$dir/SwapCore.swift"; then
    echo "MUTATION $name: the substitution matched nothing (instrument broken)"
    continue
  fi
  xcrun swiftc -target arm64-apple-macos14.0 -swift-version 5 -parse-as-library -framework Foundation -framework Security \
    "$dir/SwapCore.swift" "$HERE/SwapCoreTests.swift" -o "$dir/swapcore-tests" 2>"$dir/compile.log"
  rc=0
  "$dir/swapcore-tests" "$dir" >"$dir/run.log" 2>&1 || rc=$?
  fails="$(grep -c '^FAIL' "$dir/run.log" || true)"
  if ((rc != 0)) && grep -qF -- "$must" "$dir/run.log"; then
    red=$((red + 1))
    echo "MUTATION $name: red as required (exit $rc, $fails FAIL lines; includes '$must')"
  else
    echo "MUTATION $name: NOT red as required (exit $rc, $fails FAIL lines)"
  fi
  grep '^FAIL' "$dir/run.log" | sed 's/^/    /'
  tail -1 "$dir/run.log" | sed 's/^/    /'
done
echo "mutations: $red of $total went red"
((red == total))
