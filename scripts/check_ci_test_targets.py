#!/usr/bin/env python3
"""Check that CI runs every integration test target and skips none silently.

Cargo builds each top-level `.rs` file under `tests/` as its own test target.
A new file that no workflow step names never runs in CI, and a bare
`#[ignore]` hides a test from every run that omits `--ignored`. Both pass
silently, so this check fails on them.

Usage: check_ci_test_targets.py [TESTS_DIR] [WORKFLOW_FILE]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

DEFAULT_TESTS_DIR = "tests"
DEFAULT_WORKFLOW = ".github/workflows/ci.yml"
BARE_IGNORE = "#[ignore]"


def missing_targets(tests_dir: Path, workflow_text: str) -> list[str]:
    stems = sorted(path.stem for path in tests_dir.glob("*.rs"))
    return [
        f"{stem}: no `--test {stem}` step in the CI workflow"
        for stem in stems
        if not re.search(rf"--test\s+{re.escape(stem)}(?=\s|$)", workflow_text)
    ]


def bare_ignores(tests_dir: Path) -> list[str]:
    findings = []
    for path in sorted(tests_dir.rglob("*.rs")):
        for number, line in enumerate(path.read_text().splitlines(), start=1):
            if line.strip() == BARE_IGNORE:
                findings.append(f"{path}:{number}: {BARE_IGNORE} without a reason")
    return findings


def main(argv: list[str]) -> int:
    tests_dir = Path(argv[0] if len(argv) > 0 else DEFAULT_TESTS_DIR)
    workflow = Path(argv[1] if len(argv) > 1 else DEFAULT_WORKFLOW)
    findings = missing_targets(tests_dir, workflow.read_text()) + bare_ignores(tests_dir)
    for finding in findings:
        print(finding)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
