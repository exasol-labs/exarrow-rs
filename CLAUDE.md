@AGENTS.md

## Spec Recording Rules

- Never auto-merge a spec delta for `code-quality/core` during `/speq:record` — always confirm with the user first. This feature's delta history has had unmarked `## Background` edits (normative content outside the marked `## Scenarios` the record procedure merges), so recording it silently risks losing or misapplying requirements. Ask before merging; it is fine to record other deltas from the same plan while excluding this one (see `specs/_recorded/2026-07-29-add-unit-test-coverage` for a plan recorded this way).
