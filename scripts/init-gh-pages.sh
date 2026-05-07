#!/usr/bin/env bash
# Run once per clone of beebeeb-io/desktop to bootstrap the gh-pages branch
# that serves https://releases.beebeeb.io/latest.json.
#
# After running this:
#   1. Push: git push origin gh-pages
#   2. In GitHub repo settings → Pages: source = gh-pages branch, custom
#      domain = releases.beebeeb.io
#   3. Add DNS: releases.beebeeb.io CNAME beebeeb-io.github.io
#
# The publish-release.yml workflow takes over from there — every published
# release rewrites latest.json on this branch.

set -euo pipefail

if git rev-parse --verify gh-pages >/dev/null 2>&1; then
  echo "gh-pages branch already exists locally — aborting." >&2
  exit 1
fi

git checkout --orphan gh-pages
git rm -rf . >/dev/null

cat > latest.json <<'JSON'
{
  "version": "0.0.0",
  "notes": "Placeholder — first real release will overwrite this.",
  "pub_date": "2026-05-07T00:00:00Z",
  "platforms": {}
}
JSON

echo "releases.beebeeb.io" > CNAME

cat > index.html <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>Beebeeb releases</title>
<h1>Beebeeb desktop releases</h1>
<p>Auto-update manifest: <a href="/latest.json">/latest.json</a></p>
<p>Downloads: <a href="https://github.com/beebeeb-io/desktop/releases">github.com/beebeeb-io/desktop/releases</a></p>
HTML

git add latest.json CNAME index.html
git commit -m "init gh-pages: serve releases.beebeeb.io/latest.json"

echo
echo "gh-pages branch created locally. Next:"
echo "  git push origin gh-pages"
echo "  git checkout main"
