#!/usr/bin/env bash
# run.sh <probe.ps1> <out dir>: the probe in the VM desktop, then the VM's logs copied out. Run under the vm401 lock.
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CFG=$HOME/.config/bb-build-vm/ssh_config; out=$2; mkdir -p $out
$HERE/vmrun.sh $1 $out --timeout 3300; rc=$?
scp -q -F $CFG 'bb-build-win11:C:/bb/rec/chapters.log' $out/chapters.log
scp -q -r -F $CFG 'bb-build-win11:C:/Users/bb/AppData/Local/io.beebeeb.app.test/logs' $out/
exit $rc
