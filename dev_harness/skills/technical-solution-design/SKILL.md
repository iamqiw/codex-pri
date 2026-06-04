---
name: technical-solution-design
description: Create or review a technical solution design for secondary development, based on requirements and existing code. Use when the user asks for 技术方案设计, 方案设计, technical design, implementation design, or architecture design.
---

# Technical Solution Design

Use this skill to produce a technical solution design for secondary development. The output must be grounded in the requirement document and the existing codebase. Do not describe a greenfield architecture unless the user explicitly asks for it.

## Workflow

1. Read the requirement source. If the source is absent, extract the requirement from the conversation and mark assumptions explicitly.
2. Inspect the existing code paths before proposing changes. Prefer `rg`, repository tests, existing API definitions, schema files, message definitions, and dependency manifests.
3. Identify the involved entities and current ownership boundaries before defining new logic.
4. Produce a design that separates current behavior, proposed changes, compatibility constraints, and verification scope.
5. Mark unknowns as `待确认` instead of filling gaps with unsupported assumptions.

## Required Output Structure

Use the following sections unless the user provides a stricter template:

```markdown
## 1. 需求简述

## 2. 涉及实体

## 3. 核心链路流程图

## 4. 状态变更图

## 5. 现有关联代码分析

## 6. 代码变更范围

## 7. 新逻辑定义
### 7.1 实体定义
### 7.2 接口定义
### 7.3 数据库定义
### 7.4 消息定义

## 8. 变更影响和测试范围

## 9. 新增配置和对外依赖说明

## 10. 高级主题
### 10.1 设计决策记录
### 10.2 迁移与回滚方案
### 10.3 并发与幂等
### 10.4 错误处理和降级策略
### 10.5 观测性
### 10.6 安全与权限影响

## 11. 待确认问题
```

## Section Guidance

- `需求简述`: State the business goal, triggering condition, expected result, non-goals, and explicit constraints.
- `涉及实体`: List domain entities, persisted entities, transport DTOs, jobs, commands, events, external systems, and user-visible surfaces.
- `核心链路流程图`: Use Mermaid `flowchart` when possible. Include caller, service/module boundaries, persistence, async messages, and external systems.
- `状态变更图`: Use Mermaid `stateDiagram-v2` when the feature has lifecycle state. If no lifecycle state exists, state `无独立状态机` and explain the reason.
- `现有关联代码分析`: Cite concrete files, modules, functions, types, API routes, database migrations, message handlers, and tests. Distinguish direct dependencies from incidental references.
- `代码变更范围`: List exact files or expected modules to change, grouped by layer. Include files that must not be changed if the repository rules define boundaries.
- `新逻辑定义`: Cover entities, interfaces, database, and messages. Interfaces include HTTP APIs, CLI commands, trait methods, function contracts, RPC methods, background jobs, event topics, and tool calls.
- `变更影响和测试范围`: Describe compatibility impact, migration impact, rollback considerations, observability impact, performance risk, security risk, and required tests.
- `新增配置和对外依赖说明`: Include config keys, defaults, environment variables, feature flags, service credentials, rate limits, timeouts, retry policy, and operational ownership.
- `设计决策记录`: Record key choices with `问题`, `选项`, `决策`, `理由`, and `代价`. Include rejected options when they affect review or future maintenance.
- `迁移与回滚方案`: Cover database, configuration, protocol, and state-machine changes. If no migration is required, state `无迁移`.
- `并发与幂等`: Cover duplicate requests, retries, out-of-order messages, concurrent writes, locking strategy, unique constraints, and idempotency keys.
- `错误处理和降级策略`: Define error classes, return semantics, retry policy, user-visible messages, log levels, and fallback behavior for external dependencies, message consumers, and background jobs.
- `观测性`: Define logs, metrics, traces, audit records, alerts, dashboards, and troubleshooting entry points.
- `安全与权限影响`: Cover authentication, authorization, input validation, sensitive data, privilege escalation risk, and audit requirements. If no security impact exists, state the evidence.
- `待确认问题`: Include only unresolved decisions that affect correctness, compatibility, scope, or rollout.

## Content Levels

- Required baseline: `需求简述`, `涉及实体`, `核心链路流程图`, `状态变更图`, `现有关联代码分析`, `代码变更范围`, `新逻辑定义`, `变更影响和测试范围`, and `新增配置和对外依赖说明`.
- Advanced topics: expand only when the feature has the corresponding risk surface. If an advanced topic is not applicable, keep one short `不适用` statement with source type.
- Always expand `迁移与回滚方案` when database schema, persisted data, configuration, protocol, or state definitions change.
- Always expand `并发与幂等` when write operations, retries, async messages, background jobs, or external callbacks are involved.
- Always expand `错误处理和降级策略` when external dependencies, async processing, CLI/API errors, or user-visible failure states are involved.
- Always expand `观测性` when the change affects production operation, debugging, auditability, or incident response.
- Always expand `安全与权限影响` when the feature touches user data, credentials, permissions, input boundaries, or cross-tenant/project access.

## Design Decision Format

Use this table for decision records:

```markdown
| ID | 来源类型 | 问题 | 选项 | 决策 | 理由 | 代价 |
| --- | --- | --- | --- | --- | --- | --- |
```

## Traceability Constraints

- Every conclusion must include one source type: `需求来源`, `代码证据`, `推断`, or `待确认`.
- Prefer stable IDs for all major outputs:
  - Requirements: `REQ-001`
  - Features: `FR-001`
  - States: `ST-001`
  - Interfaces: `IF-001`
  - Test cases referenced from downstream documents: `TC-001`
- Technical solution design must map requirement-design IDs to implementation surfaces:
  - `功能点 -> 代码变更`
  - `功能点 -> 接口`
  - `功能点 -> 状态`
  - `功能点 -> 数据`
- Requirement design is the upstream source for `目标`, `功能点`, and `验收标准`.
- Test case design must map technical-solution IDs to test cases and risks.
- When no upstream ID exists, create a stable ID and keep it consistent within the document.

## Quality Constraints

- Every proposed change must map to a requirement or to an existing code constraint.
- Do not omit non-HTTP interfaces. Internal traits, commands, messages, scheduled jobs, and database contracts are part of the design surface.
- Prefer concrete file references over generic layer names.
- Keep diagrams consistent with the textual flow and state descriptions.
- If the codebase contradicts the requirement, state the conflict and propose options with tradeoffs.
