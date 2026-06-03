# Git Branch Rules

## Version branches

Version branch names must follow:

```text
version/yyyymmdd_v{x.y}_{goal}
```

Example:

```text
version/20260604_v0.1_stateless
```

## Worktree branches

Worktree branch names under a version branch must follow:

```text
version/yyyymmdd_v{x.y}_{goal}_wt_{worktree_goal}
```

Example:

```text
version/20260604_v0.1_stateless_wt_review_docs
```
