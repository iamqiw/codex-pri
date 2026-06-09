# Stateless Codex CLI 分支变更分析说明

## 1. 分析范围

本文面向不熟悉 Rust 实现细节的读者，解释 `version/20260604_v0.1_stateless` 分支相对 `main` 的主要开发内容，以及当前工作区未提交改动对该分支的补充。

分析对象包括：

- 需求文档：`dev_harness/design/versions/20260604_v0.1_stateless/stateless-codex-cli-requirements-design.md`
- 技术方案：`dev_harness/develop/versions/20260604_v0.1_stateless/stateless-codex-cli-technical-solution-design.md`
- 分支相对 `main` 的代码变更
- 当前工作区未提交的补充实现

当前分支相对 `main` 的整体变更规模约为 129 个文件、1.4 万行新增。它不是单点 bug fix，而是一组围绕“无状态云端运行入口”的协议、存储、执行边界、测试和设计文档变更。当前未提交改动又在此基础上补了真实 `request/run -> turn/start` 执行链路、MySQL ThreadStore 和跨节点 resume 验证。

## 2. 一句话结论

该分支已经完成了无状态服务的核心骨架：Request 外层资源、云端状态存储、幂等、writer lease、事件游标、严格只读 Runtime 约束、MySQL 存储、app-server wrapper 接入，以及当前补充的真实 Turn 执行和跨节点 Thread 恢复。

但它还不是完整覆盖原设计的最终产品形态。主要未完成点包括：运行中 OwnerLease 心跳和超时调度、严格的 lease-aware Thread canonical append、完整事件日志投影、ConfigSnapshot/StateDbMetadata 完整持久化、`request/run` 直接创建新 Thread 的真实执行路径、CLI `codex exec` 到云端 wrapper 的集成，以及若干 app-server 列表/搜索类 ThreadStore 能力。

## 3. 变更分层

### 3.1 设计与工程治理层

分支新增了 `dev_harness` 目录，用来沉淀需求、架构、技术方案、测试用例和技能规则。与本需求直接相关的文档包括：

- `stateless-codex-cli-requirements-design.md`：定义业务目标、实体、状态机、验收标准和非目标。
- `stateless-service-feasibility-analysis.md`：分析当前 Codex CLI/app-server 的会话、状态和执行模型是否支持无状态化。
- `stateless-codex-cli-technical-solution-design.md`：定义 Request wrapper、CloudStateStore、MySQL lease、只读 Runtime 和改造范围。
- `stateless-codex-cli-test-cases.md`：给出 request、lease、只读 Runtime 等测试方向。

这部分的价值是把“Codex CLI 服务化”拆成可实现的边界：请求状态、线程状态、执行状态、事件状态和运行时权限不再混在一个本地 CLI 进程里。

### 3.2 协议层

新增 `codex-cloud-wrapper-protocol` crate，负责定义内部云端 wrapper 的数据结构。可以把它理解为“云端服务调用 Codex 的内部接口契约”。

关键对象：

- `RequestStatus`：包含 `queued`、`running`、`cancelling`、`completed`、`failed`、`cancelled`、`interrupted`、`owner_timed_out`、`lease_lost`、`turn_timed_out`。
- `RequestRunParams` / `RequestRunResponse`：提交一次用户输入，返回 `requestId`、`threadId`、事件游标和 writer lease。
- `RequestReadParams` / `RequestReadResponse`：读取 Request 当前状态。
- `RequestCancelParams`：取消 Request。
- `RequestTerminalParams`：内部系统写入终态。
- `AppendThreadItemsWithLeaseParams`：带 lease、fencing token 和幂等 key 的线程 item 追加请求。
- `RuntimeCapabilityProfile`：表达只读 Runtime 下禁用文件写入、apply_patch、本机命令执行、进程启动和 sub-agent。

同时 app-server protocol 的 schema 和 TypeScript 产物也被更新，使这些类型可以被协议层引用和校验。

当前状态：协议模型基本覆盖了需求中的 Request 外层资源、终态分类、事件游标和 writer lease。它仍是内部 wrapper 形态，不等于已经形成正式公开 app-server v2 API。

### 3.3 云端状态存储层

新增 `codex-cloud-state` crate，负责 Request、事件、writer lease 和 MySQL 存储。

已实现能力：

- `CloudStateStore` trait：定义云端状态存储边界。
- `InMemoryCloudStateStore`：用于单进程测试和轻量验证。
- `MysqlCloudStateStore`：基于 MySQL 的持久化实现。
- `CloudRequestService`：把底层存储包装成 `request/run`、`request/read`、`request/events/list`、`request/cancel`、`request/terminal` 等业务方法。
- Request 幂等：同一 `callerId + threadId + idempotencyKey` 重复提交会返回已有 Request；输入不同会触发冲突。
- Request 事件游标：`request/queued`、`request/running`、终态事件等按 cursor 读取。
- Thread writer lease：同线程执行互斥、writer owner token、fencing token、append 幂等和 thread version。

当前未提交改动又补充了：

- `RequestRecord.turn_id` 字段。
- MySQL 表 `cloud_requests.turn_id` 及迁移。
- 按 `turn_id` 反查 Request 的能力。
- 将真实 Turn 完成/中断事件同步回 Request 终态。

当前状态：Request 状态存储和 writer lease 的基础已经完成。设计中更完整的 OwnerLease、ConfigSnapshot、StateDbMetadata 仍未形成完整持久化闭环。

### 3.4 app-server wrapper 接入层

分支新增 `CloudWrapperRequestProcessor`，让 app-server 可以处理内部 wrapper 请求。可以把它理解为 app-server 内部的“云端 Request 控制器”。

已接入的方法包括：

- `request/run`
- `request/read`
- `request/cancel`
- `request/terminal`
- `request/events/list`
- `thread/items/list`
- `thread/appendWithLease`

当前未提交改动补齐了一个关键缺口：过去 `request/run` 更偏状态包装，只能创建 Request、拿到 lease、进入 running；现在当 Request 进入 `running` 且尚无 `turnId` 时，会继续调用真实 `turn/start` 执行模型轮次，并将生成的 `turnId` 写回 Request。

简化后的调用链如下：

```text
client request/run
  -> CloudRequestService.run
  -> 创建或幂等命中 Request
  -> 获取 ThreadWriterLease
  -> Request 进入 running
  -> app-server TurnRequestProcessor.start_turn_for_cloud_request
  -> 确认 Thread 已加载；若未加载且 cloud_runtime enabled，则从 ThreadStore 恢复
  -> 调用现有 turn_start_inner
  -> 真实模型 Turn 开始
  -> 回写 request.turnId
  -> Turn 完成/中断事件反向更新 Request 终态
```

这个改动使 `request/run` 从“状态控制 API”推进为“能实际驱动 Codex 执行”的入口。

### 3.5 Thread 跨节点恢复层

当前未提交改动新增 `MysqlCloudThreadStore`，实现 `codex_thread_store::ThreadStore`。

它新增两张 MySQL 表：

- `cloud_threads`：保存 Thread 的创建参数、元数据 patch、归档时间和更新时间。
- `cloud_thread_rollout_items`：保存 Thread rollout items，也就是恢复上下文所需的历史项目。

它支持：

- 创建 Thread。
- resume Thread。
- append rollout items。
- 读取 Thread。
- 读取完整 history。
- list active threads。
- archive / unarchive。
- 更新 metadata。

app-server 在 `cloud_runtime.enabled = true` 且 `state_store = "mysql"` 时，会尝试把 ThreadStore 切换为 `MysqlCloudThreadStore`。这样节点 A 完成第一轮对话后，节点 B 可以从 MySQL 读取同一个 Thread 的历史，再启动下一轮 Turn。

当前状态：跨节点 resume 的核心路径已经补上，且有真实 MySQL 跨节点测试覆盖。限制是 `MysqlCloudThreadStore` 目前是最小可用实现，`search_threads`、`list_turns`、`list_items`、`read_thread_by_rollout_path` 仍返回 unsupported；`append_items` 也没有把 CloudStateStore 的 writer lease/fencing 参数直接纳入 ThreadStore trait。

### 3.6 core 层

`codex-core` 中新增 `ThreadManager::resume_thread_from_store`。它做的事情是：

1. 根据 `threadId` 从当前配置的 ThreadStore 读取 Thread。
2. 要求读取完整 history。
3. 把 StoredThread 转成 core 可恢复的 initial history。
4. 复用现有 `resume_thread_with_history` 启动 `CodexThread`。

这是一处必要但较小的 core 改动。云端 MySQL 细节没有放进 `codex-core`，符合“不扩大 codex-core”的架构原则。

### 3.7 严格只读 Runtime 层

分支已经实现多处只读 Runtime 防线：

- `fs/writeFile`、`fs/createDirectory`、`fs/remove`、`fs/copy` 在 cloud read-only profile 下拒绝。
- `process/spawn` 在 cloud read-only profile 下拒绝。
- `command/exec` 在 cloud read-only profile 下必须走远程 read-only exec-server；没有远程环境时拒绝，不回退到 app-server 本机。
- `apply_patch` 工具和拦截到的 patch 命令在 cloud read-only profile 下拒绝。
- sub-agent、agent jobs、部分 MCP 可写工具入口在 cloud read-only profile 下禁用或拒绝。
- hooks 在 cloud read-only profile 下跳过，避免 hook 脚本绕过只读边界写本地环境。
- exec-server 增加 read-only scratch/environment 相关能力。

当前状态：这部分是分支里完成度较高的能力。仍需注意的是“只读”是多入口组合约束；后续如果新增工具、hook、MCP 或 app-server RPC，需要继续接入同一 profile 判断，否则可能出现绕过路径。

### 3.8 测试层

分支新增大量测试，覆盖：

- wrapper 协议序列化和状态转换。
- InMemory / MySQL CloudStateStore 行为。
- Request 幂等、事件 cursor、terminal 状态。
- writer lease、fencing token、append 幂等、thread version 冲突。
- cloud runtime 下 fs/process/command_exec 只读限制。
- apply_patch、hooks、sub-agent、MCP 等只读限制。
- app-server wrapper processor 行为。

当前未提交改动新增了一个被 `#[ignore]` 标记的真实 MySQL 跨节点集成测试：

```text
request_run_executes_and_resumes_real_turn_across_mysql_app_server_nodes
```

它的验证重点不是“能创建 Request”，而是：

1. 启动两个不同 `CODEX_HOME` 的 app-server 节点。
2. 两个节点使用同一个 MySQL 数据库。
3. 节点 A 创建真实 Thread，并通过 `request/run` 执行第一轮真实 Turn。
4. Request 最终变成 `completed`，并持久化 `turnId`。
5. 节点 B 对同一 `threadId` 调用 `request/run`。
6. 节点 B 能恢复节点 A 的历史并执行第二轮真实 Turn。
7. 第二轮发给 Responses API 的请求体包含第一轮用户输入、第一轮 assistant 输出和第二轮用户输入。

这组断言能证明跨节点上下文恢复确实发生，而不是只复用了同一个 thread id。

## 4. 当前实现对需求的覆盖情况

| 需求点 | 当前覆盖情况 | 说明 |
| --- | --- | --- |
| 单次 Request 提交 | 部分完成 | wrapper 协议和 app-server 入口已存在；当前补充后可真实启动 Turn。 |
| Request 幂等 | 已完成基础能力 | 基于 caller、thread、idempotencyKey 去重，并能检测不同输入冲突。 |
| 请求状态查询 | 已完成基础能力 | `request/read` 可查状态、`turnId` 和事件游标。 |
| 事件游标读取 | 已完成基础能力 | Request 级事件已支持 cursor list；完整模型事件投影仍有限。 |
| 同线程串行化 | 部分完成 | CloudStateStore 有 writer lease；但 ThreadStore append 本身尚未显式 lease-aware。 |
| 任意节点 resume | 核心路径已补齐 | 当前 MySQL ThreadStore + core resume + 真实跨节点测试覆盖了主要 happy path。 |
| 取消 Request | 部分完成 | 可写 cancelled 终态；运行中 owner 路由和更细竞态处理仍有限。 |
| Turn 终态同步 Request | 当前补齐 | `TurnComplete` / `TurnAborted` 会通过 `turnId` 更新 Request 终态。 |
| OwnerLease 心跳和超时 | 未完整实现 | 状态枚举存在，但缺少独立 owner lease 表、心跳、扫描和超时终态调度。 |
| WriterLease fencing | 部分完成 | CloudStateStore append 具备 fencing；ThreadStore canonical append 未直接绑定 fencing。 |
| ConfigSnapshot | 未完整实现 | 协议和设计有概念，代码没有完整持久化模型。 |
| StateDbMetadata | 未完整实现 | 还没有替代本地 SQLite state_db 的完整云端实现。 |
| 严格只读 Runtime | 大部分完成 | fs/process/command/apply_patch/hooks/sub-agent 等主要入口已加防线。 |
| `codex exec` 云端入口 | 未完整实现 | 当前仍主要是 app-server wrapper；CLI 到 wrapper 的产品化路径未完成。 |
| 运行中跨实例接管 | 非目标 | 需求明确首版不要求恢复运行中的 Turn。 |

## 5. 当前未提交改动的价值

当前未提交改动解决的是这个分支中最关键的可用性断点：`request/run` 必须真的触发 Codex Turn，而不是只在云端状态里创建一个 running Request。

具体价值：

- `request/run` 能启动真实 `turn/start`。
- Request 持久化 `turnId`，后续可把 Turn 生命周期和 Request 生命周期关联起来。
- Turn 完成或中断后，Request 终态自动更新。
- app-server 能在 cloud runtime + MySQL 下使用 MySQL ThreadStore。
- 节点 B 能恢复节点 A 写入 MySQL 的 Thread history。
- 增加真实 MySQL 双节点测试，验证跨节点上下文恢复。

这使分支从“协议和状态骨架”向“可执行的无状态服务雏形”前进了一步。

## 6. 主要风险与限制

### 6.1 MySQL ThreadStore 是最小实现

`MysqlCloudThreadStore` 目前只满足真实 turn 执行和 resume 的核心路径。列表、搜索和 turn/item 分页能力不完整。若上层 UI 或客户端依赖这些 ThreadStore 方法，可能遇到 unsupported。

此外，`append_items` 通过读取当前最大 sequence 再插入新 sequence。这个路径没有显式携带 writer lease、fencing token 或 append idempotency key。只要上层保证同一 Thread 同时只有一个 writer，风险可控；如果未来绕过 `CloudStateStore` writer lease 直接并发 append，就可能出现竞争或版本语义不完整。

### 6.2 cloud runtime MySQL ThreadStore 配置失败时会 fallback

app-server 在 cloud runtime + MySQL 模式下创建 `MysqlCloudThreadStore` 失败时，目前会 warning 并 fallback 到配置默认 ThreadStore。

这对开发环境更宽容，但对生产无状态语义有风险：配置错了也可能退回本地 ThreadStore，导致跨节点 resume 不成立。生产形态更合理的策略通常是 fail fast。

### 6.3 `request/run` 直接创建新 Thread 的路径仍不完整

协议允许 `threadId` 为空或 `createThread = true`。`CloudRequestService` 会生成一个逻辑 thread id。但当前 app-server 真实执行路径会调用 `start_turn_for_cloud_request`，它需要 Thread 已存在或能从 store 恢复。

因此当前可靠路径是：先通过 `thread/start` 创建真实 Thread，再对该 `threadId` 调用 `request/run`。直接 `request/run(createThread=true)` 创建并执行真实 Thread 仍需要补齐。

### 6.4 Request 事件不等于完整 Turn 事件流

当前 Request 事件记录了 queued、running、completed、failed 等状态事件。真实模型输出、工具调用细节、token 使用等完整 Turn 事件仍主要走现有 app-server listener/ThreadStore 路径。若需求要求 `request/events/list` 能恢复完整流式输出，需要继续做事件投影或统一 EventLogStore。

### 6.5 OwnerLease 尚未形成闭环

设计要求 owner 实例定期心跳，超时后将 Request 标记为 `owner_timed_out`，并释放 writer lease 供后续请求继续执行。当前代码中已有状态和 terminal signal，但缺少独立 owner lease 生命周期、后台扫描器、心跳续期和超时处理。

### 6.6 ConfigSnapshot 和 StateDbMetadata 仍是架构缺口

无状态服务要求后续请求不能依赖某个 app-server 进程的启动配置或本地 SQLite state_db。当前分支已经有 cloud runtime 配置和 MySQL Request/Thread 存储，但没有完整实现每次 Request 的 ConfigSnapshot，也没有把 state_db metadata 完整迁到云端。

## 7. 建议后续收尾顺序

1. 先把当前未提交改动完成工程收尾：`just fmt`、相关 crate 测试、dependency lock 更新、`just fix -p ...`。
2. 将 MySQL ThreadStore 配置失败从 fallback 调整为 cloud runtime 生产模式 fail fast，至少在测试中覆盖该行为。
3. 补 `request/run(createThread=true)` 的真实 Thread 创建路径。
4. 将 Thread canonical append 与 writer lease/fencing 的关系收紧，避免存在绕过 CloudStateStore 的写入路径。
5. 增加 OwnerLease 表、心跳、超时扫描和终态写入。
6. 明确 `request/events/list` 的目标：只做 Request 状态事件，还是承载完整 Turn 输出事件；若是后者，需要补 EventLogStore 投影。
7. 补 ConfigSnapshot 和 StateDbMetadata 的最小可用模型。
8. 再考虑 CLI `codex exec` 通过云端 wrapper 执行的产品化入口。

## 8. 给非 Rust 读者的模块地图

| 路径 | 可以理解为 | 本分支做了什么 |
| --- | --- | --- |
| `codex-rs/cloud-wrapper-protocol` | 云端 wrapper 的接口定义 | 定义 Request、状态、事件、lease、只读 runtime 能力。 |
| `codex-rs/cloud-state` | 云端状态数据库访问层 | 实现 Request/事件/lease 的内存和 MySQL 存储。 |
| `codex-rs/app-server/src/request_processors/cloud_wrapper_processor.rs` | wrapper 请求处理器 | 把 app-server 请求转成 CloudRequestService 调用。 |
| `codex-rs/app-server/src/message_processor.rs` | app-server 总路由 | 接入 `request/run`，当前补充了真实 Turn 启动。 |
| `codex-rs/app-server/src/request_processors/turn_processor.rs` | Turn 执行入口 | 当前补充了 cloud request 使用的内部 start turn 方法和按 store 恢复 Thread。 |
| `codex-rs/cloud-state/src/mysql_thread_store.rs` | MySQL Thread 历史存储 | 当前新增，用于跨节点恢复 Thread history。 |
| `codex-rs/core/src/thread_manager.rs` | core Thread 生命周期管理 | 当前新增从 ThreadStore 读取 history 并恢复 Thread 的方法。 |
| `codex-rs/app-server/src/request_processors/fs_processor.rs` | app-server 文件 API | cloud read-only 下拒绝写操作。 |
| `codex-rs/app-server/src/request_processors/command_exec_processor.rs` | app-server 命令 API | cloud read-only 下强制远程 read-only exec-server。 |
| `codex-rs/core/src/tools/handlers/apply_patch.rs` | 模型 patch 工具 | cloud read-only 下拒绝写文件。 |
| `codex-rs/core/src/hook_runtime.rs` | hook 执行层 | cloud read-only 下跳过 hooks，避免脚本写本地。 |

## 9. 当前完成度判断

如果验收口径是“证明无状态 Request 可以用 MySQL 在两个 app-server 节点之间恢复 Thread 并继续真实执行”，当前未提交改动已经接近完成该口径，并且新增的真实 MySQL ignored test 正是围绕这个口径设计。

如果验收口径是需求文档中的完整首版系统，包括 OwnerLease、ThreadWriterLease 全路径 fencing、完整事件日志、ConfigSnapshot、StateDbMetadata、`codex exec` 云端入口和生产级配置失败策略，则当前分支仍是阶段性实现，不应标记为全部完成。

建议把当前分支定位为：

```text
无状态云端 Request/Thread/ReadOnly Runtime 的第一阶段实现。
已具备真实执行与跨节点恢复主链路。
仍需补齐租约超时、完整状态快照、完整事件投影和 CLI 产品入口。
```
