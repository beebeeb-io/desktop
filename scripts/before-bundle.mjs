#!/usr/bin/env node
// Tauri's beforeBundleCommand. A real script file (not an inline `node -e "..."` string in
// tauri.conf.json) so quoting behaves identically on every OS — Windows' cmd.exe mangles nested
// quotes in inline command strings differently than bash/sh.
import { execFileSync } from "node:child_process";

if (process.platform === "darwin") {
  execFileSync("scripts/prepare-macos-provisioning-profiles.sh", { stdio: "inherit" });
  execFileSync("scripts/build-fileprovider-extension.sh", { stdio: "inherit" });
}
