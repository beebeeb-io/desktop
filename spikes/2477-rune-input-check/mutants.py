#!/usr/bin/env python3
"""Shows that every test of the probe can fail: each mutant removes ONE piece of Rune wiring from src/lib.rs,
the tests run, and the tests that went red are counted.

Run it in a checkout that is nobody's working tree (the build VM's): it rewrites src/lib.rs in place and puts the
original back after every mutant. It exits non-zero when a mutant does not apply exactly once, when the unmutated
run is not green, when a mutant fails to compile, or when a test is red on no mutant.

    python3 mutants.py            # last line: "mutants: N applied, N killed; tests: T, T red on at least one"
"""
import pathlib
import re
import subprocess
import sys

SRC = pathlib.Path(__file__).parent / "src" / "lib.rs"

# (name, exact text in src/lib.rs, replacement). Each text must occur exactly `count` times (default 1).
MUTANTS = [
    ("no App::on_ime", ".on_ime(on_ime)", ""),
    ("no El::ime_cursor", "                .ime_cursor()\n", ""),
    ("no El::on_key", "        .on_key(on_key)\n", ""),
    ("no El::on_drag", "        .on_drag(on_drag);", "        ;"),
    ("canvas not focusable", "        .focusable(true)\n        .on_key(on_key)", "        .focusable(false)\n        .on_key(on_key)"),
    ("no Tx::copy_rich", "tx.copy_rich(ClipboardContent::new(selected.clone()).with_html(html));", "let _ = html;"),
    ("copy without the HTML form", ".with_html(html));", ");\n            let _ = html;"),
    ("no Tx::read_clipboard", "Key::Command('v') => tx.read_clipboard(", "Key::Command('\\u{1}') => tx.read_clipboard("),
    ("no AppSpec::poll", "poll: Some(poll),", "poll: None,"),
    ("no App::on_focus", ".on_focus(on_window_focus)", ""),
    ("text field without on_edit", "                .on_edit(|s: &mut Probe| &mut s.field),\n", ""),
    ("on_key without Tx::instant", "    tx.instant();\n    s.log\n        .push(format!(\"key", "    s.log\n        .push(format!(\"key"),
    ("pre-edit cursor ignored", "else { *cursor };", "else { None };"),
    ("click count ignored", "d.clicks >= 2", "d.clicks >= 99"),
    ("no El::on_drawn_layout", ".on_drawn_layout(|s: &mut Probe, r| s.caret_drawn = Some(r))", ""),
]


def run_tests():
    """Returns (compiled, {test name: passed})."""
    out = subprocess.run(["cargo", "test", "--locked", "--test", "input"], capture_output=True, text=True)
    text = out.stdout + out.stderr
    results = {m.group(1): m.group(2) == "ok" for m in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED)$", text, re.M)}
    if not results:
        sys.stdout.write(text[-3000:])
    return bool(results), results


def main():
    original = SRC.read_text()
    compiled, base = run_tests()
    if not compiled or not all(base.values()) or not base:
        print(f"unmutated run is not green: {sum(base.values())} of {len(base)} passed")
        return 1
    print(f"unmutated: {len(base)} of {len(base)} passed")
    killed_by = {name: [] for name in base}
    killed = 0
    try:
        for name, old, new in MUTANTS:
            if original.count(old) != 1:
                print(f"mutant '{name}' applies {original.count(old)} times, not once")
                return 1
            SRC.write_text(original.replace(old, new))
            compiled, results = run_tests()
            if not compiled:
                print(f"mutant '{name}' did not compile")
                return 1
            if set(results) != set(base):
                print(f"mutant '{name}' ran {len(results)} tests, not {len(base)}")
                return 1
            red = sorted(t for t, ok in results.items() if not ok)
            killed += bool(red)
            for t in red:
                killed_by[t].append(name)
            print(f"mutant '{name}': {len(red)} of {len(results)} red: {', '.join(red) or 'NONE'}")
    finally:
        SRC.write_text(original)
    never = sorted(t for t, names in killed_by.items() if not names)
    for t, names in sorted(killed_by.items()):
        print(f"test {t}: red on {len(names)} mutant(s)")
    print(f"mutants: {len(MUTANTS)} applied, {killed} killed; tests: {len(base)}, {len(base) - len(never)} red on at least one")
    return 1 if never or killed != len(MUTANTS) else 0


if __name__ == "__main__":
    sys.exit(main())
