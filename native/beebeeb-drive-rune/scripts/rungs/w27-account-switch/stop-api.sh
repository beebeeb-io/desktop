#!/usr/bin/env bash
# stop my own test API: the process that listens on 47851 (it runs from my stack dir), until the port is closed
for i in 1 2 3 4 5 6 7 8 9 10; do
  pid=$(ss -ltnpH 'sport = :47851' 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | head -1)
  [ -z "$pid" ] && break
  [ "$(readlink /proc/$pid/cwd)" = "${W27_STACK:?}"/run ] || { echo "47851 is not mine (pid $pid)"; exit 1; }
  kill $pid; sleep 1
done
curl -s -o /dev/null --max-time 2 http://127.0.0.1:47851/health && { echo "still up"; exit 1; }; echo "api down"
