#!/usr/bin/env python3
"""Check that CI runs every integration test target and skips none silently.

Cargo builds each top-level `.rs` file under `tests/` as its own test target.
A new file that no active `integration-tests` command names never runs in CI, and a bare
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
INTEGRATION_JOB = "integration-tests"
RUN_KEY = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
SHELL_COMMENT = re.compile(r"(^|\s)#.*$")
INERT_COMMAND = re.compile(r"^(echo|printf)(\s|$)")


def job_lines(workflow_text: str, job: str) -> list[str]:
    """Return the lines of one top-level job, found without a YAML dependency."""
    lines = workflow_text.splitlines()
    start = next((i for i, line in enumerate(lines) if line.rstrip() == f"  {job}:"), None)
    if start is None:
        return []
    end = next(
        (i for i in range(start + 1, len(lines)) if re.match(r"  \S", lines[i])),
        len(lines),
    )
    return lines[start + 1 : end]


def run_commands(lines: list[str]) -> list[str]:
    """Return the active shell command lines of every `run:` step in `lines`."""
    commands = []
    i = 0
    while i < len(lines):
        match = RUN_KEY.match(lines[i])
        i += 1
        if not match:
            continue
        indent, value = len(match.group(1)), match.group(2)
        if re.match(r"^[|>][+-]?\d*\s*$", value):
            body = []
            while i < len(lines) and (
                not lines[i].strip() or len(lines[i]) - len(lines[i].lstrip()) > indent
            ):
                body.append(lines[i])
                i += 1
        else:
            body = [value]
        for line in body:
            command = SHELL_COMMENT.sub("", line).strip()
            if command and not INERT_COMMAND.match(command):
                commands.append(command)
    return commands


def missing_targets(tests_dir: Path, workflow_text: str) -> list[str]:
    stems = sorted(path.stem for path in tests_dir.glob("*.rs"))
    commands = "\n".join(run_commands(job_lines(workflow_text, INTEGRATION_JOB)))
    return [
        f"{stem}: no `--test {stem}` command in the `{INTEGRATION_JOB}` job"
        for stem in stems
        if not re.search(rf"--test\s+{re.escape(stem)}(?=\s|$)", commands)
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
