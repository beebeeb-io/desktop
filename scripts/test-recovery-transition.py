#!/usr/bin/env python3
"""Execute the exact recovery command with held verifier/credential/runner doubles.
No source rewriting beyond extracting the command; both Windows cfg and the
reacquisition race are exercised. Real auth lease tests run in Cargo separately.
"""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / 'src-tauri/src/lib.rs').read_text()
a = source.index('async fn desktop_unlock_with_recovery_phrase(')
body = source[a:source.index('\n}', a) + 2]
fixture = (root / 'scripts/fixtures/recovery-transition.rs').read_text()
with tempfile.TemporaryDirectory(prefix='recovery-transition-') as temp:
    path = Path(temp) / 'fixture.rs'
    binary = Path(temp) / 'fixture'
    path.write_text(fixture + '\n' + body)
    subprocess.run(['rustc', '--edition=2024', '--test', '-Aexplicit_builtin_cfgs_in_flags',
                    '--cfg', 'target_os="windows"', str(path), '-o', str(binary)], check=True)
    subprocess.run([str(binary), '--nocapture'], check=True)
