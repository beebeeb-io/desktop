#!/usr/bin/env bash
# vmrun.sh <script.ps1> <outdir> [run-in-desktop opts]: my own tunnel (not the shared connection): VM 3051 -> my API 47851, VM 47736 -> the code sink 47826; PIN pipe; the script in the desktop session. Run under with-lock vm401.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "${BB_WORKSPACE:?set BB_WORKSPACE to the beebeeb.io workspace}" || exit 1
CFG=$HOME/.config/bb-build-vm/ssh_config
script=$1; out=$2; shift 2
for f in $HERE/lib25.ps1 $HERE/lib26.ps1; do scp -q -F $CFG $f "bb-build-win11:C:/bb/w25/$(basename $f)"; done
ssh -F $CFG -o BatchMode=yes -o ControlPath=none -o ControlMaster=no -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -N -R 127.0.0.1:3051:127.0.0.1:47851 -R 127.0.0.1:47736:127.0.0.1:47826 bb-build-win11 &
T=$!
ssh -F $CFG -o BatchMode=yes bb-build-win11 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\bb\rec\pin-pipe-rec.ps1 -Serve 1 -WaitSec 1500' < $HOME/.config/bb-build-vm/hello-pin > ${W27_OUT:-/tmp}/pinpipe.log 2>&1 &
P=$!
trap 'kill $T $P 2>/dev/null' EXIT
sleep 4
kill -0 $T 2>/dev/null || { echo "TUNNEL FAILED: my forward is not up"; exit 9; }
ssh -F $CFG -o BatchMode=yes bb-build-win11 'curl.exe -s -o NUL -w "tunnel health %{http_code}" http://127.0.0.1:3051/health'; echo
scripts/coord/build-vm/run-in-desktop.sh "$script" --out "$out" "$@"
