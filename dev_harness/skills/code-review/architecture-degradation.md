# Architecture Degradation Review

## Objective

Identify whether the change weakens architectural boundaries, increases coupling, or introduces structures that reduce maintainability.

## Review checks

- Verify that new responsibilities are placed in the correct module, crate, package, or service boundary.
- Check whether the change adds logic to central modules when a narrower owner exists.
- Check whether public APIs expose implementation details or unstable internal concepts.
- Check whether the change introduces cyclic dependencies, hidden data flow, or duplicated orchestration paths.
- Check whether the change bypasses existing abstractions that already encode required invariants.
- Check whether new concepts require an architecture note under `dev_harness/arch/versions/`.

## Finding format

- Boundary affected.
- Evidence from changed files.
- Long-term maintainability risk.
- Required correction or acceptable constraint.
