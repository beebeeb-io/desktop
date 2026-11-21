#!/usr/bin/env bash
# Fails when a tracked file names internal machines, hosts or people. This repo is public.
# Allowed: .github/CODEOWNERS (GitHub handles) and Windows SDK version strings (10.0.NNNNN.N).
set -u
cd "$(git rev-parse --show-toplevel)"
PATTERN='vm ?40[01]|legion|\bGuus\b|with-lock|bb-build-win11|ssh_config|\b10\.0\.5\.[0-9]|proxmox|bb-legion'
hits=$(git grep -nIiE "$PATTERN" -- . ':!.github/CODEOWNERS' ':!scripts/check-infra-names.sh')
if [ -n "$hits" ]; then
  echo "$hits"
  echo "check-infra-names: internal infra names found above" >&2
  exit 1
fi
echo "check-infra-names: ok"
