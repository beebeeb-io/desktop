#!/usr/bin/env python3
"""
Check that release.yml's `release` and `publish-manifest` jobs cannot be
silently skipped after a successful `build` on a workflow_dispatch from main.

A job with `needs: X` and no explicit status function in its `if:` gets
GitHub's implicit success(), which evaluates the entire transitive dependency
chain.  When verify-branch-point is skipped (dispatch from main), that chain
contains a skipped job, so the implicit success() returns false and the job is
skipped even though build itself succeeded (task 2458).

The fix: use an explicit status function (`!cancelled()`) together with an
explicit `needs.build.result == 'success'` check in the `release` job's `if:`.

Usage:
  python3 scripts/test-release-workflow-ifs.py           # check release.yml
  python3 scripts/test-release-workflow-ifs.py --self-test  # prove the red case

Truth line: release-workflow-ifs: N passed, 0 failed
"""
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
WORKFLOW = ROOT / '.github/workflows/release.yml'

EXPECTED_CHECKS = 6

passed = 0
failed = 0


def _ok(name: str) -> None:
    global passed
    passed += 1


def _fail(name: str, msg: str = '') -> None:
    global failed
    failed += 1
    extra = f': {msg}' if msg else ''
    print(f'FAIL [{name}]{extra}', file=sys.stderr)


def check(name: str, cond: bool, msg: str = '') -> None:
    _ok(name) if cond else _fail(name, msg)


def extract_job_if(text: str, job_name: str) -> str | None:
    """
    Extract the if: expression for a named top-level job in a GitHub Actions
    workflow YAML file.  Returns the expression as a single normalised string
    (stripped of ${{ }}, extra whitespace and newlines), or None when the job
    or its if: key is not present.

    Handles both inline `if: <expr>` and block scalar `if: >-\\n  ${{ … }}`.
    """
    # Find the job header at exactly 2-space indentation.
    job_header = re.compile(r'^  ' + re.escape(job_name) + r'\s*:', re.MULTILINE)
    m = job_header.search(text)
    if not m:
        return None

    # Slice out this job's section: everything from its header up to the next
    # sibling key at the same indentation level.
    job_start = m.start()
    rest = text[m.end():]
    next_sibling = re.search(r'^  [A-Za-z0-9]', rest, re.MULTILINE)
    job_block = rest[: next_sibling.start()] if next_sibling else rest

    # Find `if:` at 4-space indentation within this block.
    if_line = re.compile(r'^    if:\s*(.*)$', re.MULTILINE)
    im = if_line.search(job_block)
    if not im:
        return None

    value = im.group(1).strip()

    # Block scalar indicator: collect the indented continuation lines.
    if value in ('>', '>-', '|', '|-'):
        lines = []
        for line in job_block[im.end():].split('\n'):
            if line.startswith('      '):   # 6-space base indent for block content
                lines.append(line.strip())
            elif line.strip() == '':
                continue
            else:
                break
        value = ' '.join(lines)

    # Normalise: strip ${{ }}, collapse whitespace.
    value = re.sub(r'\$\{\{|\}\}', '', value)
    value = ' '.join(value.split())
    return value


def has_explicit_status(expr: str) -> bool:
    """True when expr contains an explicit GitHub Actions status function."""
    return bool(re.search(r'(always|success|failure|cancelled)\(\)|!cancelled\(\)', expr))


def run_checks(text: str) -> None:
    # --- release job ---
    release_if = extract_job_if(text, 'release')
    if release_if is None:
        _fail('release-job-found', 'release job or its if: missing in release.yml')
        # Can't run per-field checks without the value; count them as failures.
        for _ in range(4):
            _fail('release-job-found (skipped sub-check)', 'parent check failed')
    else:
        check(
            'release.if has explicit status function',
            has_explicit_status(release_if),
            f'got: {release_if!r}'
        )
        check(
            "release.if checks needs.build.result == 'success'",
            "needs.build.result == 'success'" in release_if,
            f'got: {release_if!r}'
        )
        check(
            "release.if keeps workflow_dispatch guard",
            "github.event_name == 'workflow_dispatch'" in release_if,
            f'got: {release_if!r}'
        )
        check(
            "release.if keeps publish_existing guard",
            '!inputs.publish_existing' in release_if,
            f'got: {release_if!r}'
        )

    # --- publish-manifest job ---
    pm_if = extract_job_if(text, 'publish-manifest')
    if pm_if is None:
        _fail('publish-manifest-job-found', 'publish-manifest job or its if: missing in release.yml')
        _fail('publish-manifest-job-found (skipped sub-check)', 'parent check failed')
    else:
        check(
            'publish-manifest.if has explicit status function',
            has_explicit_status(pm_if),
            f'got: {pm_if!r}'
        )
        check(
            "publish-manifest.if checks needs.release.result == 'success'",
            "needs.release.result == 'success'" in pm_if,
            f'got: {pm_if!r}'
        )


def self_test() -> None:
    """Prove the count guard can go red."""
    good_log = f'release-workflow-ifs: {EXPECTED_CHECKS} passed, 0 failed\n'
    bad_log_fewer = f'release-workflow-ifs: {EXPECTED_CHECKS - 1} passed, 0 failed\n'
    bad_log_failed = f'release-workflow-ifs: {EXPECTED_CHECKS} passed, 1 failed\n'
    empty_log = ''

    def assert_log_ok(log: str) -> bool:
        m = re.search(r'^release-workflow-ifs: (\d+) passed, (\d+) failed$', log, re.MULTILINE)
        if not m:
            return False
        return int(m.group(1)) == EXPECTED_CHECKS and int(m.group(2)) == 0

    assert assert_log_ok(good_log), 'self-test: good log was rejected'
    assert not assert_log_ok(bad_log_fewer), 'self-test: fewer-checks log was accepted'
    assert not assert_log_ok(bad_log_failed), 'self-test: failed log was accepted'
    assert not assert_log_ok(empty_log), 'self-test: empty log was accepted'
    print(f'self-test OK: count guard rejected 3 bad logs and accepted the good one')


if __name__ == '__main__':
    if len(sys.argv) == 2 and sys.argv[1] == '--self-test':
        self_test()
        sys.exit(0)

    text = WORKFLOW.read_text()
    run_checks(text)
    print(f'release-workflow-ifs: {passed} passed, {failed} failed')
    sys.exit(0 if failed == 0 else 1)
