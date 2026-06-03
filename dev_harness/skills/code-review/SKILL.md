# Code Review Skill

## Purpose

Run code review from multiple independent perspectives. Each perspective must produce findings with file references, risk classification, and required remediation when applicable.

## Review perspectives

- Architecture degradation: see `architecture-degradation.md`.
- Security: see `security.md`.
- Performance risk: see `performance-risk.md`.
- Coding standards: see `coding-standards.md`.

## Output requirements

- Findings must be grouped by perspective.
- Findings must distinguish confirmed defects from risks and assumptions.
- Each actionable finding must include the affected file, location when available, impact, and recommended change.
- If a perspective has no findings, state that explicitly and record remaining assumptions.
