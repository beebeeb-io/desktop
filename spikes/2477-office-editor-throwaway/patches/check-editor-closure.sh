#!/usr/bin/env bash
# Fails when a crate that can open a connection is in what the editor links from WordCraft.
# Run after every upstream bump: re-apply the patch (git apply --directory=wordcraft patches/*.patch), then this.
set -eu
cd "$(dirname "$0")/../wordcraft"
# EDITOR_FLAGS= (empty) checks the default features instead: that run must FAIL, it proves the check can.
FLAGS=${EDITOR_FLAGS---no-default-features}
NET='reqwest ureq hyper h2 curl isahc attohttpc tiny_http tungstenite rustls native-tls openssl quinn socket2 mio tokio async-std smol async-io'
bad=0
for target in x86_64-pc-windows-msvc x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-gnu; do
  names=$(cargo tree -e normal --prefix none --target "$target" -p wordcraft-engine $FLAGS -f '{p}' | awk '{print $1}' | sort -u)
  n=$(echo "$names" | wc -l)
  for c in $NET; do
    if echo "$names" | grep -qx "$c"; then echo "FAIL [$target]: $c is in the editor's closure"; bad=1; fi
  done
  echo "[$target] wordcraft-engine $FLAGS: $n packages checked"
done
[ $bad -eq 0 ] && echo "editor closure: no network crate" || exit 1
