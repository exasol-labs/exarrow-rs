#!/usr/bin/env python3
"""Reduce an lcov report to production code only.

`cargo llvm-cov` instruments `#[cfg(test)] mod ... { ... }` blocks like any
other code, so unit tests count themselves in the coverage denominator and
inflate the reported percentage. Rust's `#[coverage(off)]` attribute would
exclude them at the source level but is nightly-only, and this crate pins
stable Rust, so the exclusion happens here instead: every line entry that
falls inside a `#[cfg(test)]` module is dropped and the per-file totals are
recomputed.

Two commands:

    strip  --input lcov-unit.info --output lcov-unit-production.info \\
           --summary coverage-summary.json
    check  --summary coverage-summary.json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable, Sequence

DEFAULT_MIN_TOTAL = 80.0
DEFAULT_MIN_PER_FILE = 50.0

# Files exempt from the per-file floor. Each entry needs a reason: the floor
# exists to stop coverage rotting, so a silent blanket lowering is worse than a
# named, reviewable exception. See AGENTS.md for the removal criteria.
PER_FILE_FLOOR_EXEMPTIONS: tuple[str, ...] = (
    # 48.4% production-only. Its uncovered lines are the CSV write paths, which
    # are reachable without I/O and so should be unit-tested; the exemption is
    # a temporary acknowledgement of that debt, not a permanent carve-out.
    # Delete this entry once the file clears the floor on its own.
    "src/export/csv.rs",
)

RAW_STRING_PREFIX = re.compile(r'(?:b|c)?r(#*)"')
CFG_ATTRIBUTE = re.compile(r"#\s*\[\s*cfg\s*\(")
MODULE_HEADER = re.compile(
    r"(?:pub\s*(?:\([^)]*\)\s*)?)?mod\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*"
)
CRATE_ROOT_STEMS = frozenset({"mod", "lib", "main"})
IDENTIFIER_CHARACTER = re.compile(r"[A-Za-z0-9_]")


class ProductionSourceMissingError(Exception):
    """An lcov record names a source file that cannot be read."""


class CoverageFloorError(Exception):
    """Production coverage is below a configured floor."""


# --------------------------------------------------------------------------- #
# Rust source scanning
# --------------------------------------------------------------------------- #


def mask_non_code(source: str) -> str:
    """Blank comments and literals, preserving every byte offset and newline.

    Braces inside strings, char literals and comments must not be counted when
    brace-matching a module body, and `cfg(test)` inside a string must not be
    mistaken for an attribute. Blanking rather than removing keeps line and
    column numbers of the surviving code identical to the original.
    """
    characters = list(source)
    length = len(source)

    def blank(start: int, end: int) -> None:
        for index in range(start, min(end, length)):
            if characters[index] != "\n":
                characters[index] = " "

    position = 0
    while position < length:
        character = source[position]
        if character == "/" and source.startswith("//", position):
            end = source.find("\n", position)
            end = length if end == -1 else end
            blank(position, end)
            position = end
        elif character == "/" and source.startswith("/*", position):
            end = _end_of_block_comment(source, position)
            blank(position, end)
            position = end
        elif character in "rbc" and _raw_string_starts_at(source, position):
            end = _end_of_raw_string(source, position)
            blank(position, end)
            position = end
        elif character == '"':
            end = _end_of_string(source, position)
            blank(position, end)
            position = end
        elif character == "'":
            end = _end_of_char_literal(source, position)
            if end is None:
                position += 1
            else:
                blank(position, end)
                position = end
        else:
            position += 1
    return "".join(characters)


def _end_of_block_comment(source: str, start: int) -> int:
    depth = 1
    position = start + 2
    length = len(source)
    while position < length and depth > 0:
        if source.startswith("/*", position):
            depth += 1
            position += 2
        elif source.startswith("*/", position):
            depth -= 1
            position += 2
        else:
            position += 1
    return position


def _raw_string_starts_at(source: str, start: int) -> bool:
    if start > 0 and IDENTIFIER_CHARACTER.match(source[start - 1]):
        return False
    return RAW_STRING_PREFIX.match(source, start) is not None


def _end_of_raw_string(source: str, start: int) -> int:
    match = RAW_STRING_PREFIX.match(source, start)
    assert match is not None
    terminator = '"' + match.group(1)
    end = source.find(terminator, match.end())
    return len(source) if end == -1 else end + len(terminator)


def _end_of_string(source: str, start: int) -> int:
    position = start + 1
    length = len(source)
    while position < length:
        if source[position] == "\\":
            position += 2
            continue
        if source[position] == '"':
            return position + 1
        position += 1
    return length


def _end_of_char_literal(source: str, start: int) -> int | None:
    """End offset of a char literal at `start`, or None for a lifetime."""
    if source[start + 1 : start + 2] == "\\":
        position = _end_of_escape_sequence(source, start + 2)
        if source[position : position + 1] == "'":
            return position + 1
        return None
    if source[start + 2 : start + 3] == "'":
        return start + 3
    return None


def _end_of_escape_sequence(source: str, start: int) -> int:
    kind = source[start : start + 1]
    if kind == "u" and source[start + 1 : start + 2] == "{":
        closing = source.find("}", start + 1)
        return len(source) if closing == -1 else closing + 1
    if kind == "x":
        return start + 3
    return start + 1


def cfg_selects_test_only(predicate: str) -> bool:
    """Whether a `cfg(...)` predicate is satisfied only in a test build.

    `not(test)` guards production code and `any(test, unix)` also holds outside
    tests, so neither may be stripped; `all(test, unix)` only ever holds under
    test and may be.
    """
    predicate = predicate.strip()
    if predicate == "test":
        return True
    match = re.fullmatch(r"(not|all|any)\s*\((.*)\)", predicate, re.DOTALL)
    if match is None:
        return False
    operator, arguments = match.group(1), _split_top_level(match.group(2))
    if operator == "not":
        return False
    if operator == "all":
        return any(cfg_selects_test_only(argument) for argument in arguments)
    return bool(arguments) and all(
        cfg_selects_test_only(argument) for argument in arguments
    )


def _split_top_level(arguments: str) -> list[str]:
    parts: list[str] = []
    depth = 0
    current = ""
    for character in arguments:
        if character == "," and depth == 0:
            parts.append(current)
            current = ""
            continue
        if character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
        current += character
    if current.strip():
        parts.append(current)
    return parts


@dataclass(frozen=True)
class _TestModule:
    """A `#[cfg(test)]` module, either inline (`mod m { .. }`) or out of line."""

    attribute_offset: int
    name: str
    body_offset: int | None


def find_test_module_ranges(source: str) -> list[tuple[int, int]]:
    """Inclusive 1-based line ranges of every inline `#[cfg(test)]` module."""
    masked = mask_non_code(source)
    line_of = _line_index(masked)
    ranges: list[tuple[int, int]] = []
    for module in _find_test_modules(masked):
        if module.body_offset is None:
            continue
        body_end = _matching_bracket(masked, module.body_offset, "{", "}")
        last = len(masked) - 1 if body_end is None else body_end - 1
        ranges.append((line_of(module.attribute_offset), line_of(last)))
    return _merge_overlapping(ranges)


def test_only_module_paths(source_file: str, source: str) -> set[str]:
    """Candidate paths of the files holding this file's out-of-line test modules.

    `#[cfg(test)] mod support;` puts test code in a separate file, which
    `cargo llvm-cov` reports as its own record. Nothing inside that file marks
    it as test code, so the exclusion has to come from the declaring file.
    """
    parent = _module_directory(source_file)
    paths: set[str] = set()
    for module in _find_test_modules(mask_non_code(source)):
        if module.body_offset is None:
            paths.add(f"{parent}/{module.name}.rs")
            paths.add(f"{parent}/{module.name}/mod.rs")
    return paths


def _module_directory(source_file: str) -> str:
    path = Path(source_file)
    is_root = path.stem in CRATE_ROOT_STEMS
    return (path.parent if is_root else path.with_suffix("")).as_posix()


def _find_test_modules(masked: str) -> Iterable[_TestModule]:
    for attribute in CFG_ATTRIBUTE.finditer(masked):
        predicate_end = _matching_bracket(masked, attribute.end() - 1, "(", ")")
        if predicate_end is None:
            continue
        if not cfg_selects_test_only(masked[attribute.end() : predicate_end - 1]):
            continue
        header = _module_header(masked, predicate_end)
        if header is None:
            continue
        name, end = header
        if masked[end : end + 1] == "{":
            yield _TestModule(attribute.start(), name, end)
        elif masked[end : end + 1] == ";":
            yield _TestModule(attribute.start(), name, None)


def _module_header(masked: str, position: int) -> tuple[str, int] | None:
    """Name of the module declared after the attribute closing at `position`."""
    while True:
        position = _skip_whitespace(masked, position)
        if masked[position : position + 1] != "]":
            return None
        position = _skip_whitespace(masked, position + 1)
        if masked[position : position + 1] != "#":
            break
        bracket = masked.find("[", position)
        if bracket == -1:
            return None
        closed = _matching_bracket(masked, bracket, "[", "]")
        if closed is None:
            return None
        position = closed - 1
    header = MODULE_HEADER.match(masked, position)
    return None if header is None else (header.group("name"), header.end())


def _skip_whitespace(text: str, position: int) -> int:
    while position < len(text) and text[position].isspace():
        position += 1
    return position


def _matching_bracket(text: str, start: int, opening: str, closing: str) -> int | None:
    """Offset just past the bracket matching the one at `start`."""
    depth = 0
    for position in range(start, len(text)):
        if text[position] == opening:
            depth += 1
        elif text[position] == closing:
            depth -= 1
            if depth == 0:
                return position + 1
    return None


def _line_index(text: str) -> Callable[[int], int]:
    starts = [0]
    for position, character in enumerate(text):
        if character == "\n":
            starts.append(position + 1)

    def line_of(offset: int) -> int:
        low, high = 0, len(starts) - 1
        while low < high:
            middle = (low + high + 1) // 2
            if starts[middle] <= offset:
                low = middle
            else:
                high = middle - 1
        return low + 1

    return line_of


def _merge_overlapping(ranges: list[tuple[int, int]]) -> list[tuple[int, int]]:
    merged: list[tuple[int, int]] = []
    for start, end in sorted(ranges):
        if merged and start <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(merged[-1][1], end))
        else:
            merged.append((start, end))
    return merged


def _contains(ranges: Sequence[tuple[int, int]], line: int) -> bool:
    return any(start <= line <= end for start, end in ranges)


# --------------------------------------------------------------------------- #
# lcov rewriting
# --------------------------------------------------------------------------- #

RECOMPUTED_KEYS = ("LF", "LH", "BRF", "BRH", "FNF", "FNH")


@dataclass(frozen=True)
class RecordTotals:
    lines_found: int
    lines_hit: int
    branches_found: int
    branches_hit: int
    functions_found: int
    functions_hit: int

    def as_lcov(self) -> dict[str, int]:
        return {
            "LF": self.lines_found,
            "LH": self.lines_hit,
            "BRF": self.branches_found,
            "BRH": self.branches_hit,
            "FNF": self.functions_found,
            "FNH": self.functions_hit,
        }


def strip_lcov(
    report: str, read_source: Callable[[str], str]
) -> tuple[str, dict[str, object]]:
    """Rewrite `report` without test-module lines; return it with a summary."""
    records = list(_split_records(report))
    sources = {
        record.source_file: _read(read_source, record.source_file)
        for record in records
        if record.source_file is not None
    }
    excluded = set().union(
        *(test_only_module_paths(path, source) for path, source in sources.items()),
        set(),
    )
    output: list[str] = []
    files: list[dict[str, object]] = []
    for record in records:
        if record.source_file is None:
            output.extend(record.lines)
            continue
        if record.source_file in excluded:
            continue
        ranges = find_test_module_ranges(sources[record.source_file])
        kept, totals = _strip_record(record.lines, ranges)
        output.extend(kept)
        files.append(
            {
                "file": record.source_file,
                "lines_found": totals.lines_found,
                "lines_hit": totals.lines_hit,
                "percentage": _percentage(totals.lines_hit, totals.lines_found),
            }
        )
    return "".join(line + "\n" for line in output), _summarise(files)


def _summarise(files: list[dict[str, object]]) -> dict[str, object]:
    lines_found = sum(int(entry["lines_found"]) for entry in files)
    lines_hit = sum(int(entry["lines_hit"]) for entry in files)
    return {
        "lines_found": lines_found,
        "lines_hit": lines_hit,
        "percentage": _percentage(lines_hit, lines_found),
        "files": sorted(files, key=_coverage_order),
    }


def _coverage_order(entry: dict[str, object]) -> tuple[float, str]:
    percentage = entry["percentage"]
    ranking = 101.0 if percentage is None else float(percentage)
    return (ranking, str(entry["file"]))


def _percentage(hit: int, found: int) -> float | None:
    return None if found == 0 else round(100.0 * hit / found, 4)


def _read(read_source: Callable[[str], str], path: str) -> str:
    try:
        return read_source(path)
    except (KeyError, OSError, UnicodeDecodeError) as error:
        raise ProductionSourceMissingError(
            f"cannot read source file {path!r} referenced by the lcov report: {error}"
        ) from error


@dataclass(frozen=True)
class _Record:
    source_file: str | None
    lines: list[str]


def _split_records(report: str) -> Iterable[_Record]:
    current: list[str] = []
    source_file: str | None = None
    for line in report.splitlines():
        current.append(line)
        if line.startswith("SF:"):
            source_file = line[3:].strip()
        elif line.strip() == "end_of_record":
            yield _Record(source_file, current)
            current, source_file = [], None
    if current:
        yield _Record(source_file, current)


def _strip_record(
    lines: list[str], ranges: Sequence[tuple[int, int]]
) -> tuple[list[str], RecordTotals]:
    dropped_functions = {
        _function_name(line)
        for line in lines
        if line.startswith("FN:") and _contains(ranges, _entry_line(line))
    }
    kept = [
        line for line in lines if not _is_dropped(line, ranges, dropped_functions)
    ]
    totals = _totals(kept)
    replacements = totals.as_lcov()
    return [_rewrite_total(line, replacements) for line in kept], totals


def _is_dropped(
    line: str, ranges: Sequence[tuple[int, int]], dropped_functions: set[str]
) -> bool:
    if line.startswith(("DA:", "BRDA:", "FN:")):
        return _contains(ranges, _entry_line(line))
    if line.startswith("FNDA:"):
        return _function_name(line) in dropped_functions
    return False


def _rewrite_total(line: str, replacements: dict[str, int]) -> str:
    key = line.split(":", 1)[0]
    return f"{key}:{replacements[key]}" if key in RECOMPUTED_KEYS else line


def _totals(lines: list[str]) -> RecordTotals:
    line_entries = [line for line in lines if line.startswith("DA:")]
    branch_entries = [line for line in lines if line.startswith("BRDA:")]
    function_entries = [line for line in lines if line.startswith("FN:")]
    execution_counts = [line for line in lines if line.startswith("FNDA:")]
    return RecordTotals(
        lines_found=len(line_entries),
        lines_hit=sum(1 for line in line_entries if _execution_count(line) > 0),
        branches_found=len(branch_entries),
        branches_hit=sum(1 for line in branch_entries if _branch_taken(line)),
        functions_found=len(function_entries),
        functions_hit=sum(1 for line in execution_counts if _execution_count(line) > 0),
    )


def _entry_line(line: str) -> int:
    return int(_fields(line)[0])


def _function_name(line: str) -> str:
    """The mangled name of an `FN:<line>,<name>` or `FNDA:<count>,<name>` entry."""
    return line.split(":", 1)[1].split(",", 1)[1]


def _execution_count(line: str) -> int:
    field = _fields(line)[1] if line.startswith("DA:") else _fields(line)[0]
    return 0 if field == "-" else int(field)


def _branch_taken(line: str) -> bool:
    taken = _fields(line)[3]
    return taken != "-" and int(taken) > 0


def _fields(line: str) -> list[str]:
    return line.split(":", 1)[1].split(",")


# --------------------------------------------------------------------------- #
# Floor enforcement
# --------------------------------------------------------------------------- #


def check_floors(
    summary: dict[str, object],
    min_total: float,
    min_per_file: float,
    exempt_files: Sequence[str],
) -> None:
    """Raise CoverageFloorError when the total or any file is below its floor."""
    problems: list[str] = []
    total = summary["percentage"]
    if total is None or float(total) < min_total:
        problems.append(
            f"total production line coverage {_format(total)} "
            f"is below the {min_total:.1f}% floor"
        )
    exempt = set(exempt_files)
    for entry in summary["files"]:  # type: ignore[union-attr]
        percentage = entry["percentage"]
        if percentage is None or str(entry["file"]) in exempt:
            continue
        if float(percentage) < min_per_file:
            problems.append(
                f"{entry['file']} at {_format(percentage)} "
                f"is below the {min_per_file:.1f}% per-file floor"
            )
    if problems:
        raise CoverageFloorError("\n".join(problems))


def _format(percentage: object) -> str:
    return "n/a" if percentage is None else f"{float(percentage):.2f}%"


# --------------------------------------------------------------------------- #
# Command line
# --------------------------------------------------------------------------- #


def _relative_source_reader(repo_root: Path) -> Callable[[str], str]:
    def read(path: str) -> str:
        return (repo_root / path).read_text(encoding="utf-8")

    return read


def _relative(path: str, repo_root: Path) -> str:
    candidate = Path(path)
    if not candidate.is_absolute():
        return candidate.as_posix()
    try:
        return candidate.resolve().relative_to(repo_root).as_posix()
    except ValueError:
        return candidate.as_posix()


def _normalise_paths(report: str, repo_root: Path) -> str:
    lines = [
        f"SF:{_relative(line[3:].strip(), repo_root)}"
        if line.startswith("SF:")
        else line
        for line in report.splitlines()
    ]
    return "".join(line + "\n" for line in lines)


def _run_strip(arguments: argparse.Namespace) -> int:
    repo_root = arguments.repo_root.resolve()
    report = _normalise_paths(
        arguments.input.read_text(encoding="utf-8"), repo_root
    )
    stripped, summary = strip_lcov(report, _relative_source_reader(repo_root))
    arguments.output.write_text(stripped, encoding="utf-8")
    arguments.summary.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(
        f"production line coverage: {_format(summary['percentage'])} "
        f"({summary['lines_hit']}/{summary['lines_found']} lines)"
    )
    for entry in summary["files"][:10]:  # type: ignore[index]
        print(f"  {_format(entry['percentage']):>8}  {entry['file']}")
    return 0


def _run_check(arguments: argparse.Namespace) -> int:
    summary = json.loads(arguments.summary.read_text(encoding="utf-8"))
    try:
        check_floors(
            summary,
            min_total=arguments.min_total,
            min_per_file=arguments.min_per_file,
            exempt_files=PER_FILE_FLOOR_EXEMPTIONS,
        )
    except CoverageFloorError as error:
        print(f"production coverage floor not met:\n{error}", file=sys.stderr)
        return 1
    print(
        f"production line coverage {_format(summary['percentage'])} "
        f"meets the {arguments.min_total:.1f}% floor"
    )
    return 0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    strip = commands.add_parser("strip", help="write a production-only lcov report")
    strip.add_argument("--input", type=Path, default=Path("lcov-unit.info"))
    strip.add_argument("--output", type=Path, default=Path("lcov-unit-production.info"))
    strip.add_argument("--summary", type=Path, default=Path("coverage-summary.json"))
    strip.add_argument("--repo-root", type=Path, default=Path(__file__).parent.parent)
    strip.set_defaults(handler=_run_strip)

    check = commands.add_parser("check", help="enforce the production coverage floors")
    check.add_argument("--summary", type=Path, default=Path("coverage-summary.json"))
    check.add_argument("--min-total", type=float, default=DEFAULT_MIN_TOTAL)
    check.add_argument("--min-per-file", type=float, default=DEFAULT_MIN_PER_FILE)
    check.set_defaults(handler=_run_check)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    return int(arguments.handler(arguments))


if __name__ == "__main__":
    sys.exit(main())
