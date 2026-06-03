# Dev Harness

## Directory structure and purpose

- `arch/`: Architecture analysis, architecture decision drafts, and system boundary notes.
  - `arch/versions/`: Version-scoped architecture material.
  - `arch/archive/`: Manually archived version-scoped architecture material.
  - `arch/global/`: Global architecture documents and cross-version architecture baselines.
- `design/`: Product design notes, interaction design drafts, and user-facing workflow analysis.
  - `design/versions/`: Version-scoped design material.
  - `design/archive/`: Manually archived version-scoped design material.
  - `design/global/`: Global design documents and cross-version design baselines.
- `develop/`: Development plans, implementation notes, implementation tradeoff analysis, and local development support material.
  - `develop/versions/`: Version-scoped development process material.
  - `develop/archive/`: Manually archived version-scoped development process material.
- `engineering/`: Reusable engineering standards, scripts, and technical checks.
- `eval/`: Evaluation plans, evaluation datasets, result summaries, and quality criteria.
- `skills/`: Agent skill drafts, skill specifications, and reusable workflow instructions.
- `test/`: Test plans, test cases, test fixtures, and verification records.
  - `test/versions/`: Version-scoped test material.
  - `test/archive/`: Manually archived version-scoped test material.
  - `test/global/`: Global test documents, baseline test cases, and cross-version verification criteria.

## Git constraints

- Git branch naming must follow `engineering/GitBranchRules.md`.
- If the current branch does not follow that rule, remind the user, but do not force a branch rename or modify Git state.

## Compounding rules

- When a version branch is created, initialize same-named version subdirectories under `arch/versions/`, `design/versions/`, `develop/versions/`, and `test/versions/`.
- Version-scoped material must be placed in the corresponding version subdirectory.
- If business design or architecture design changes during development, update the corresponding design document.
- Developer-requested changes and bug-fix process records must be captured under the corresponding `develop/versions/` subdirectory.
- Version archiving is a developer-executed manual process: move version-scoped material into the corresponding `archive/` directory, then update `arch/global/`, `design/global/`, and `test/global/` from the archived version documents. If new reusable standards are identified, recommend placing them under `engineering/`.
