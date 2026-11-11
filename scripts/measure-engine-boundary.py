#!/usr/bin/env python3
"""The counts in docs/superpowers/specs/2026-11-11-native-engine-boundary.md, so anyone can re-measure them.

  scripts/measure-engine-boundary.py <dir of .rs files>      e.g. src-tauri/src, crates/beebeeb-engine/src

Method, per top-level module (a file `<name>.rs` plus everything under `<name>/`):
  lines       newline characters in its `.rs` files (what `wc -l` prints), tests and comments included
  tauri refs  matches of  \\btauri(_plugin_\\w+)?::  |  use tauri\\b  |  AppHandle  |  tauri::State
              (so a `use tauri::{A, B}` line is 1, and each later bare `AppHandle` is 1 more; `\\btauri\\b` alone
              gives a lower number because it does not count the bare type names)
  cmds        `#[tauri::command` attributes
  emits       `.emit(`, `.emit_to(` and `.emit_filter(` calls
  depends on  the 3 other top-level modules it names most often as `crate::<module>`
A test module is a top-level module named `tests` or `*_tests`.
"""
import collections
import os
import re
import sys

REFS = re.compile(r"\btauri(?:_plugin_\w+)?::|use tauri\b|AppHandle|tauri::State")


def measure(root):
    modules = collections.defaultdict(lambda: [0, 0, 0, 0, collections.Counter()])
    for directory, _, files in os.walk(root):
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(directory, name)
            top = os.path.relpath(path, root).replace(os.sep, "/").split("/")[0].removesuffix(".rs")
            text = open(path, encoding="utf-8", errors="replace").read()
            module = modules[top]
            module[0] += text.count("\n")
            module[1] += len(REFS.findall(text))
            module[2] += text.count("#[tauri::command")
            module[3] += len(re.findall(r"\.emit(?:_to|_filter)?\(", text))
            for dependency in re.findall(r"crate::([a-z_0-9]+)", text):
                if dependency != top:
                    module[4][dependency] += 1
    return modules


def main():
    modules = measure(sys.argv[1])
    is_test = lambda name: name == "tests" or name.endswith("_tests")
    product = {k: v for k, v in modules.items() if not is_test(k)}
    tests = {k: v for k, v in modules.items() if is_test(k)}
    total = lambda rows, i: sum(row[i] for row in rows.values())
    print(f"modules {len(modules)} lines {total(modules, 0)} | product {len(product)} lines {total(product, 0)} | "
          f"test modules {len(tests)} lines {total(tests, 0)}")
    print(f"tauri refs {total(modules, 1)} cmds {total(modules, 2)} emits {total(modules, 3)} | "
          f"modules with 0 tauri refs {sum(1 for m in modules.values() if m[1] == 0)}")
    for name, m in sorted(product.items(), key=lambda item: -item[1][0]):
        deps = ", ".join(d for d, _ in m[4].most_common(3)) or "-"
        print(f"| `{name}` | {m[0]:,} | {m[1]} | {m[2]} | {m[3]} | {deps} |")


if __name__ == "__main__":
    main()
