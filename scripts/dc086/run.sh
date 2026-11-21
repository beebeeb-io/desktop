#!/usr/bin/env bash
# dc-086: run.sh <n> <timeout s>: the kit to C:\bb\dc086, then probe-<n>.ps1 in the VM's desktop session.
# Call it under scripts/coord/with-lock.sh vm401.
set -u
K=/home/guus/code/bb-worktrees/desktop-dc-163-real/scripts/dc086
CFG=$HOME/.config/bb-build-vm/ssh_config; n=$1; to=$2; out=$HOME/.cache/dc163/out/run$n
mkdir -p "$out"; cd /home/guus/code/beebeeb.io || exit 1
vm() { ssh -F "$CFG" -o BatchMode=yes -o ControlPath=none -o ControlMaster=no -o LogLevel=ERROR bb-build-win11 "$@"; }
vm 'New-Item -ItemType Directory -Force C:\bb\dc086 | Out-Null' || exit 8
for f in rec-lib.ps1 ks.ps1 sampler.ps1 threads2.ps1; do scp -q -F "$CFG" -o ControlPath=none -o LogLevel=ERROR "$K/$f" "bb-build-win11:C:/bb/dc086/$f" || exit 8; done
scripts/coord/build-vm/run-in-desktop.sh "$K/probe-$n.ps1" --out "$out" --timeout "$to" > "$out/probe.log" 2>&1; rc=$?
echo "run rc=$rc" >> "$out/probe.log"; exit $rc
