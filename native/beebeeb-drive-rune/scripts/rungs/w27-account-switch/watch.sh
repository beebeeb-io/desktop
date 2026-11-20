#!/usr/bin/env bash
# watch.sh <run log>: on READY-FOR-API-DOWN / READY-FOR-API-UP in the run log, stop / start my own API
log=$1; ST=${W27_STACK:?set W27_STACK to the test stack dir}; V=$(cd "$(dirname "$0")" && pwd); down=0
for i in $(seq 1 7000); do
  grep -aq LOCKRUN_DONE $log 2>/dev/null && break
  if [ $down = 0 ] && grep -aq READY-FOR-API-DOWN $log 2>/dev/null; then echo "$(date +%T) $($V/stop-api.sh)" >> $ST/watch.out; down=1; fi
  if grep -aq READY-FOR-API-UP $log 2>/dev/null; then $V/start-api.sh; echo "$(date +%T) api started" >> $ST/watch.out; break; fi
  sleep 1; done
