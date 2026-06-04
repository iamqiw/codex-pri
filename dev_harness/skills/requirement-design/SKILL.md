---
name: requirement-design
description: Create or review a requirement design document grounded in current product behavior and existing code. Use when the user asks for 需求设计, 需求方案, requirement design, product requirement design, or PRD analysis with code feasibility.
---

# Requirement Design

Use this skill to produce a requirement design document for secondary development. The output must start from a concrete business scenario and connect product intent, current behavior, existing code constraints, and feasible delivery scope.

## Workflow

1. Establish the business scenario first. Identify actor, trigger, business context, current problem, expected business result, frequency, and boundary conditions.
2. If the business scenario is too broad or ambiguous, ask concise interactive questions before finalizing the document. Prefer questions that clarify actor, trigger, success result, exception path, data object, or operational constraint.
3. Read the requirement source or extract the requirement from the conversation. Mark missing input as `待补充`.
4. Inspect current implementation before writing feasibility conclusions. Prefer `rg`, existing routes, UI components, domain models, services, tests, schema files, config, and message definitions.
5. Separate current-state facts from assumptions and proposed behavior.
6. Identify entities, major flows, key states, and state closure before listing functions.
7. If frontend interaction exists, describe the actual user operation flow and expected UI state changes.

## Required Output Structure

Use the following sections unless the user provides a stricter template:

```markdown
## 1. 业务场景说明

## 2. 目标

## 3. 范围边界

## 4. 现状分析

## 5. 结合实际代码的可行性分析

## 6. 实体、主要流程图、关键状态和状态闭环分析
### 6.1 实体
### 6.2 主要流程图
### 6.3 关键状态
### 6.4 状态闭环分析

## 7. 功能点列表

## 8. 角色与权限

## 9. 数据口径

## 10. 前端交互操作流程

## 11. 高级主题
### 11.1 兼容性约束

## 12. 待确认问题
```

If no frontend interaction exists, keep section 10 and state `无前端交互变更`.

## Section Guidance

- `业务场景说明`: This is the foundational input. Define actor, business context, trigger, current pain point, expected business result, occurrence frequency, related data object, exception path, and operational constraints. If any of these are unknown and affect design correctness, ask the user before finalizing or mark them as `待确认`.
- `目标`: Define business objective, target user/system actor, trigger condition, expected outcome, non-goals, and success criteria.
- `范围边界`: State what is in scope for the current delivery and what is explicitly out of scope. Do not mix implementation details, technical debt cleanup, or long-term planning into current scope unless they are required for the requirement.
- `现状分析`: Describe current behavior, current user/system flow, existing data shape, known constraints, and current limitations. Cite concrete code paths when available.
- `结合实际代码的可行性分析`: Map the requirement to existing modules, functions, interfaces, storage, messages, tests, and configuration. State feasible reuse points, required extensions, incompatible assumptions, implementation risks, and alternative options.
- `实体`: Include domain entities, persisted entities, DTOs, UI state, configuration objects, jobs, messages, and external systems.
- `主要流程图`: Use Mermaid `flowchart` when possible. Include actors, UI/API/CLI entry points, service boundaries, persistence, async processing, and external dependencies.
- `关键状态`: List states, state owners, transition triggers, entry conditions, exit conditions, and invalid transitions.
- `状态闭环分析`: Verify each terminal, failure, cancellation, retry, rollback, and timeout path has a defined next state or recovery rule.
- `功能点列表`: Use stable IDs. Include feature name, description, actor, priority, input, output, impacted code area, dependency, and verifiable acceptance criteria. Acceptance criteria must be bound to each feature row.
- `角色与权限`: For user-facing behavior, define roles, permissions, visibility, allowed operations, denied operations, and permission-failure behavior.
- `数据口径`: Define field meanings, statistical rules, display rules, default values, null semantics, sorting/filtering rules, and count semantics.
- `前端交互操作流程`: Include page/entry, user action, system response, loading/empty/error states, validation, permissions, confirmation, navigation, and final visible result.
- `兼容性约束`: Cover historical data, older clients, existing configuration, existing API callers, existing workflows, and rollout constraints.
- `待确认问题`: Include only unresolved decisions that affect scope, correctness, feasibility, delivery order, or compatibility.

## Content Levels

- Required baseline: `业务场景说明`, `目标`, `范围边界`, `现状分析`, `结合实际代码的可行性分析`, `实体`, `主要流程图`, `关键状态`, `状态闭环分析`, `功能点列表`, and `验收标准`.
- Required when user-facing operations exist: `角色与权限` and `前端交互操作流程`.
- Required when the feature displays, stores, calculates, filters, sorts, or aggregates data: `数据口径`.
- Advanced topics: `兼容性约束`. Expand this section when existing data, clients, APIs, configuration, or workflows may be affected. Otherwise state `无兼容性影响` with source type.

## Interactive Scenario Clarification

When the business scenario is incomplete, ask up to three focused questions at a time. Continue only when enough information exists to distinguish scope, actors, data object, and success result. Use these prompts as needed:

- Who performs the operation, and under what business condition?
- What current problem is being solved, and what result is considered successful?
- What data object is affected, and what state should it enter after the operation?
- What exception, cancellation, duplicate operation, or timeout path must be supported?
- What existing workflow, client, configuration, or permission model must remain compatible?

## Functional List Format

Use this table unless a different format is requested:

```markdown
| ID | 来源类型 | 功能点 | 参与方 | 优先级 | 输入 | 输出 | 影响范围 | 依赖 | 验收标准 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
```

## Traceability Constraints

- Every conclusion must include one source type: `需求来源`, `代码证据`, `推断`, or `待确认`.
- Prefer stable IDs for all major outputs:
  - Requirements: `REQ-001`
  - Features: `FR-001`
  - States: `ST-001`
  - Interfaces: `IF-001`
  - Test cases referenced from downstream documents: `TC-001`
- Requirement design defines the upstream traceability baseline: `目标`, `功能点`, and `验收标准`.
- Technical solution design must map requirement-design IDs to code changes, interfaces, states, and data.
- Test case design must map requirement-design and technical-solution IDs to test cases.
- When no upstream ID exists, create a stable ID and keep it consistent within the document.

## Quality Constraints

- Do not state feasibility without citing code evidence or explicitly marking it as an assumption.
- Do not list functions before defining the relevant entities and state transitions.
- Do not omit failure, cancellation, duplicate operation, retry, timeout, and permission paths when they affect state closure.
- Keep product requirements distinct from technical implementation details, but include code feasibility because this is secondary development.
- Do not separate acceptance criteria from feature rows when the criteria can be verified per feature.
- If code behavior conflicts with the intended requirement, state the conflict and provide options with constraints.
