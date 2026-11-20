#!/usr/bin/env bash
# lockrun.sh <log> <cmd...>: with-lock vm401 per command; waits (retry every 5 s) while another holds the lock
log=$1; shift
cd "${BB_WORKSPACE:?set BB_WORKSPACE to the beebeeb.io workspace}" || exit 1
while true; do
  scripts/coord/with-lock.sh vm401 -- "$@" > "$log" 2>&1; rc=$?
  if [ $rc -ne 0 ] && [ "$(wc -l < "$log")" -le 2 ] && grep -q "is locked by" "$log"; then sleep 5; continue; fi
  break
done
echo "LOCKRUN_DONE rc=$rc" >> "$log"
