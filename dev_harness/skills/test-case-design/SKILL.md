---
name: test-case-design
description: Create or review test case designs from requirement documents and technical solution designs. Use when the user asks for 用例设计, 测试用例设计, test case design, QA case design, or test plan derived from requirements and design documents.
---

# Test Case Design

Use this skill to produce test case designs from requirement documents, technical solution designs, and existing implementation constraints. The output must connect each test area to a requirement, design decision, interface, state transition, or integration boundary.

## Workflow

1. Read the requirement document and technical solution design. If either is absent, infer from the conversation and mark missing source material as `待补充`.
2. Inspect related code, existing tests, fixtures, and test utilities when available.
3. Identify core operation flows, state transitions, boundary scenarios, data dependencies, and external dependency behavior.
4. Produce executable-oriented test cases. Avoid only describing broad categories.
5. Separate must-test cases from optional exploratory cases when scope is large.

## Required Output Structure

Use the following sections unless the user provides a stricter template:

```markdown
## 1. 核心操作流程说明

## 2. 边界场景列表

## 3. 测试数据依赖和准备方案

## 4. 测试用例

## 5. 需求/设计追踪矩阵

## 6. 自动化分层建议

## 7. 高级主题
### 7.1 回归范围
### 7.2 不可测项和替代验证
### 7.3 环境与依赖约束

## 8. 覆盖关系和剩余风险
```

## Section Guidance

- `核心操作流程说明`: Describe the primary user/system flows in execution order. Include actor, entry point, preconditions, major steps, state changes, persistence, messages, and expected output.
- `边界场景列表`: Cover permission boundaries, invalid input, missing data, duplicate submission, idempotency, timeout, retry, partial failure, concurrency, ordering, compatibility, migration, rollback, observability, and external dependency degradation.
- `测试数据依赖和准备方案`: Specify required accounts, fixtures, database rows, files, feature flags, config values, mocked services, message topics, clocks, and cleanup strategy.
- `测试用例`: Provide case ID, priority, source reference, preconditions, data setup, steps, expected result, assertion points, automation level, and test layer.
- `需求/设计追踪矩阵`: Map test cases to requirement IDs, feature IDs, interface IDs, state IDs, and risk IDs. Use the matrix to identify missing coverage.
- `自动化分层建议`: State which cases belong to unit, integration, end-to-end, contract, migration, performance, security, or manual verification. Do not model all cases as E2E.
- `回归范围`: Identify existing capabilities that must be retested, grouped by impacted module, interface, state, database table, and frontend entry.
- `不可测项和替代验证`: List scenarios that cannot be automated reliably and provide manual verification, log verification, data verification, or monitoring verification.
- `环境与依赖约束`: Define test environment, external service mocks, account permissions, data isolation, cleanup strategy, clock/time dependency, and unavailable dependencies.
- `覆盖关系和剩余风险`: Map cases to requirements, design sections, interfaces, states, and boundary scenarios. State uncovered risks explicitly.

## Content Levels

- Required baseline: `核心操作流程说明`, `边界场景列表`, `测试数据依赖和准备方案`, `测试用例`, `需求/设计追踪矩阵`, and `自动化分层建议`.
- Advanced topics: `回归范围`, `不可测项和替代验证`, and `环境与依赖约束`. Expand them when secondary-development impact, automation limits, or environment constraints affect execution. Otherwise state `无` with source type.
- Priority rules are required for every case set:
  - `P0`: Primary flow, permissions, data consistency, state closure, migration safety, and high-risk integration boundaries.
  - `P1`: Boundary conditions, recoverable failures, retries, idempotency, compatibility, and major UI error states.
  - `P2`: Experience details, low-risk compatibility paths, observability checks, and exploratory cases.

## Test Case Format

Use a table for compact case lists:

```markdown
| ID | 优先级 | 来源类型 | 需求/功能/接口/状态/风险ID | 测试层级 | 前置条件 | 数据准备 | 操作步骤 | 预期结果 | 断言点 | 自动化 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
```

Use separate detailed subsections when a case requires complex setup or multi-step verification.

## Traceability Matrix Format

Use this table for the requirement/design traceability matrix:

```markdown
| 上游ID | 类型 | 来源类型 | 覆盖用例ID | 覆盖状态 | 缺口/风险 |
| --- | --- | --- | --- | --- | --- |
```

## Traceability Constraints

- Every conclusion must include one source type: `需求来源`, `代码证据`, `推断`, or `待确认`.
- Prefer stable IDs for all major outputs:
  - Requirements: `REQ-001`
  - Features: `FR-001`
  - States: `ST-001`
  - Interfaces: `IF-001`
  - Test cases: `TC-001`
- Test case design must map test cases to upstream IDs:
  - `功能点 -> 测试用例`
  - `接口 -> 测试用例`
  - `状态 -> 测试用例`
  - `风险 -> 测试用例`
- Requirement design defines `目标`, `功能点`, and `验收标准`.
- Technical solution design maps `功能点 -> 代码变更/接口/状态/数据`.
- When no upstream ID exists, create a stable ID for the uncovered risk or inferred requirement and mark the source type as `推断` or `待确认`.

## Quality Constraints

- Do not produce test cases that cannot be traced to a requirement, design statement, interface, state transition, or risk.
- Include negative and recovery cases, not only successful paths.
- Include integration boundaries when the design mentions database writes, messages, RPC, CLI, tool calls, scheduled jobs, or external services.
- State which cases are suitable for unit, integration, end-to-end, contract, migration, performance, or security testing.
- Prefer existing repository test utilities and fixture patterns when code is available.
