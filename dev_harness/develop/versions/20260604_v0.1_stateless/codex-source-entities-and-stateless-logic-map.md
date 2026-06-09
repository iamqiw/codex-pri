# Codex 源码实体与无状态化逻辑地图

## 1. 文档目标

本文给不熟悉 Rust 和 Codex 源码的读者使用。它不讨论 Rust 语法细节，而是解释本次无状态化需求中反复出现的核心实体、它们在源码里的位置、它们之间如何调用，以及“只替换存储层”为什么仍会触碰到部分执行路径。

阅读本文时可以先记住一个简化模型：

```text
app-server 接收请求
  -> 找到或恢复 Thread
  -> 启动一次 Turn
  -> Turn 调用模型和工具
  -> 过程中产生 Event / RolloutItem
  -> ThreadStore 持久化历史
  -> 客户端读取状态或结果
```

无状态化的核心是把“后续请求必须命中同一个进程”改成“后续请求命中任意进程，都能从共享存储恢复 Thread 并继续执行”。

## 2. 总体源码分层

| 层级 | 主要目录 | 原来是否存在 | 本分支变化 | 和无状态化的关系 |
| --- | --- | --- | --- | --- |
| 协议层 | `codex-rs/app-server-protocol`、`codex-rs/cloud-wrapper-protocol` | app-server protocol 已存在；cloud-wrapper-protocol 新增 | 新增 Request、lease、event、runtime capability 协议 | 决定请求参数、响应字段、状态枚举长什么样 |
| app-server 层 | `codex-rs/app-server` | 已存在 | 新增 cloud wrapper processor；扩展 turn/start、终态同步和只读 RPC 限制 | 当前入口逻辑、turn/start、request/run 都在这里 |
| core 层 | `codex-rs/core` | 已存在 | 新增从 ThreadStore 按 threadId 恢复；扩展只读工具约束 | 真正启动 Thread/Turn，处理模型和工具 |
| thread-store 层 | `codex-rs/thread-store` | 已存在 | trait 基本沿用；新增 MySQL 实现放在 cloud-state | 替换本地存储为 MySQL/云端存储的主要接口 |
| cloud-state 层 | `codex-rs/cloud-state` | 新增 | 新增 Request/lease/event 存储；当前补充 MySQL ThreadStore | 承载 Request 状态、幂等、writer lease 和 MySQL 实现 |
| exec-server 层 | `codex-rs/exec-server` | 已存在 | 新增 read-only scratch/environment 相关能力 | 无状态云端运行时要求严格只读 |

从职责看，`ThreadStore` 只负责“Thread 历史怎么存”。它不天然负责 Request 幂等、状态查询、取消、owner 超时、事件游标等服务化语义。

源码模块变化可以先按下面这张表定位：

| 模块/文件 | 原来是否存在 | 本分支变化 |
| --- | --- | --- |
| `codex-rs/thread-store/src/store.rs` | 已存在 | trait 基本沿用；未把 lease/fencing 加进 `append_items` 参数。 |
| `codex-rs/thread-store/src/types.rs` | 已存在 | 继续作为 Thread 持久化参数和返回值定义。 |
| `codex-rs/core/src/thread_manager.rs` | 已存在 | 新增 `resume_thread_from_store`。 |
| `codex-rs/app-server/src/request_processors/turn_processor.rs` | 已存在 | 新增 `start_turn_for_cloud_request` 和 `ensure_cloud_thread_loaded`。 |
| `codex-rs/app-server/src/message_processor.rs` | 已存在 | 新增 cloud wrapper 分发；当前未提交改动把 `request/run` 桥接到真实 Turn。 |
| `codex-rs/app-server/src/bespoke_event_handling.rs` | 已存在 | Turn 完成/中断时新增 Request 终态同步。 |
| `codex-rs/cloud-wrapper-protocol` | 新增 crate | 定义 Request、lease、event、runtime capability 协议。 |
| `codex-rs/cloud-state` | 新增 crate | 定义并实现 CloudStateStore、CloudRequestService、MySQL 存储。 |
| `codex-rs/cloud-state/src/mysql_thread_store.rs` | 新增文件 | 当前未提交改动新增，用于 MySQL Thread history。 |
| `codex-rs/app-server/src/request_processors/cloud_wrapper_processor.rs` | 新增文件 | app-server 到 CloudRequestService 的适配层。 |
| `codex-rs/app-server/src/request_processors/fs_processor.rs` | 已存在 | 新增 cloud read-only 下拒绝写操作。 |
| `codex-rs/app-server/src/request_processors/process_exec_processor.rs` | 已存在 | 新增 cloud read-only 下拒绝 process/spawn。 |
| `codex-rs/app-server/src/request_processors/command_exec_processor.rs` | 已存在 | 新增 cloud read-only 下强制远程 read-only exec-server。 |
| `codex-rs/core/src/tools/handlers/apply_patch.rs` | 已存在 | 新增 cloud read-only 下拒绝 patch。 |
| `codex-rs/core/src/hook_runtime.rs` | 已存在 | 新增 cloud read-only 下跳过 hooks。 |
| `codex-rs/exec-server/src/read_only_scratch.rs` | 新增文件 | 支撑只读执行环境。 |

## 3. 核心实体地图

先看实体的新旧关系，再看每个实体细节：

| 实体/概念 | 原来是否存在 | 本分支变化 | 说明 |
| --- | --- | --- | --- |
| Thread | 已存在 | 扩展持久化方式 | 原来就是 Codex 的对话/任务会话；本分支重点是让 Thread history 可放到共享 MySQL。 |
| ThreadId | 已存在 | 继续复用 | 跨节点恢复依赖同一个 `threadId`。 |
| Turn | 已存在 | 与 Request 建立关联 | 原来就是一次模型执行；本分支新增 `request.turnId`，让服务请求能追踪真实 Turn。 |
| TurnStatus | 已存在 | 未完全替代 RequestStatus | 原有状态不足以表达 queued、cancelled、owner_timed_out、lease_lost 等请求级状态。 |
| Event | 已存在 | 新增 Request 级事件 | 原来 app-server/core 已有执行事件；本分支新增 `request/queued`、`request/running`、`request/completed` 等 Request 状态事件。 |
| RolloutItem | 已存在 | 改为可写入 MySQL ThreadStore | 原来用于本地 rollout/history 恢复；本分支用它支撑跨节点恢复。 |
| ThreadStore | 已存在 | 新增 MySQL 实现 | trait 原来就存在；新增 `MysqlCloudThreadStore`。 |
| LocalThreadStore / InMemoryThreadStore | 已存在 | 保留 | 本地和测试场景继续可用。 |
| MysqlCloudThreadStore | 新增 | 新增 | 用 MySQL 保存 `cloud_threads` 和 `cloud_thread_rollout_items`。 |
| ThreadManager | 已存在 | 新增从 store 恢复方法 | 新增 `resume_thread_from_store`，支持按 `threadId` 读取共享 history。 |
| CodexThread / Session | 已存在 | 基本复用 | 运行中的执行模型不重写；无状态化恢复的是历史，不迁移运行中 Session。 |
| Request | 新增服务化概念 | 新增 | 用来表达一次外部服务调用，不是 Codex 原始 core 对话模型里的核心对象。 |
| RequestStatus | 新增 | 新增 | 表达 queued、running、completed、failed、cancelled、owner_timed_out、lease_lost 等服务请求状态。 |
| CloudStateStore | 新增 | 新增 | 保存 Request、事件、writer lease、append 幂等结果。 |
| CloudRequestService | 新增 | 新增 | 封装 request/run/read/cancel/terminal/events/list 等业务逻辑。 |
| CloudWrapperRequestProcessor | 新增 | 新增 | app-server 内部处理 cloud wrapper 请求的 processor。 |
| ThreadWriterLease | 新增 | 新增 | 用于同 Thread 串行执行和写入 fencing。 |
| OwnerLease | 设计中新增 | 尚未完整实现 | 状态枚举和 signal 有雏形，但缺少完整心跳、表和超时扫描。 |
| ConfigSnapshot | 设计中新增 | 尚未完整实现 | 需求要求保存请求使用的配置快照，当前代码未形成完整闭环。 |
| StateDbMetadata | 设计中新增 | 尚未完整实现 | 需求要求把本地 state_db 相关元数据云端化，当前仍未完整替代。 |
| ReadOnlyRuntimeProfile | 新增/扩展配置 | 已实现主要限制 | 新增 cloud runtime 配置，并扩展多个工具入口做只读限制。 |

### 3.1 Thread

Thread 可以理解为“一段持续对话或任务会话”。同一个 Thread 下可以有多次用户输入和多轮模型回复。

源码相关位置：

- `codex-rs/thread-store/src/types.rs`
  - `CreateThreadParams`
  - `ResumeThreadParams`
  - `StoredThread`
  - `StoredThreadHistory`
- `codex-rs/core/src/thread_manager.rs`
  - `ThreadManager`
  - `start_thread_with_options`
  - `resume_thread_with_history`
  - `resume_thread_from_store`
- `codex-rs/cloud-state/src/mysql_thread_store.rs`
  - `MysqlCloudThreadStore`

Thread 的关键字段和概念：

| 概念 | 含义 |
| --- | --- |
| `thread_id` | Thread 的唯一 ID，跨请求恢复时最关键 |
| metadata | cwd、model provider、memory mode 等线程级元信息 |
| history | 已经发生过的对话和工具调用历史 |
| rollout items | Codex 用来重放和恢复 Thread 的历史记录单元 |

在本地模式下，Thread 历史主要落在本地 rollout 文件或本地状态库。无状态化后，需要把这些历史放到共享存储，例如 MySQL。

### 3.2 Turn

Turn 可以理解为“Thread 里的一次模型执行”。通常一次用户输入会触发一次 Turn。

源码相关位置：

- `codex-rs/app-server/src/request_processors/turn_processor.rs`
  - `TurnRequestProcessor`
  - `turn_start`
  - `turn_start_inner`
  - `start_turn_for_cloud_request`
- `codex-rs/app-server-protocol/src/protocol/v2/turn.rs`
  - `TurnStartParams`
  - `TurnStartResponse`
  - `TurnStatus`

Turn 的关键语义：

| 概念 | 含义 |
| --- | --- |
| `turn/start` | app-server 现有入口，用于在某个 Thread 上启动一次模型执行 |
| `TurnStatus` | 现有 Turn 状态，通常是 completed、interrupted、failed、inProgress |
| `turn_id` | 一次模型执行的 ID，可用于把 Request 和真实执行关联起来 |

无状态化时，关键不是新建一个 Turn 概念，而是让任意节点在收到请求时，能够先恢复 Thread，再启动 Turn。

### 3.3 Request

Request 是本次无状态化新增或强化的服务化概念。它不是 Codex 原本对话模型里的核心对象，而是云端服务调用方的一次提交。

源码相关位置：

- `codex-rs/cloud-wrapper-protocol/src/request.rs`
  - `RequestRunParams`
  - `RequestRunResponse`
  - `RequestRecord`
  - `RequestStatus`
  - `TerminalSignal`
- `codex-rs/cloud-state/src/request_store.rs`
  - `RequestRecord`
  - `CloudStateStore`
  - `CreateRequestParams`
- `codex-rs/cloud-state/src/request_service.rs`
  - `CloudRequestService`
- `codex-rs/app-server/src/request_processors/cloud_wrapper_processor.rs`
  - `CloudWrapperRequestProcessor`

Request 和 Turn 的区别：

| 对象 | 关注点 | 例子 |
| --- | --- | --- |
| Request | 服务调用生命周期 | 幂等、排队、状态查询、取消、事件游标 |
| Turn | 模型执行生命周期 | 调用模型、执行工具、产出 assistant 回复 |

一个常见映射是：

```text
一次 request/run
  -> 创建一个 Request
  -> 获取同 Thread writer lease
  -> 启动一个 Turn
  -> Request.turnId = Turn.id
  -> Turn 完成后 Request 进入 completed
```

如果选择“不新增入口层”，也可以把 Request 放到外部调度系统中；但只要需求里还保留幂等、状态查询、取消和事件游标，就必须有某个系统承担 Request 语义。

### 3.4 Event

Event 是状态变化或执行输出的可读取记录。

源码相关位置：

- `codex-rs/cloud-wrapper-protocol/src/event.rs`
  - `RequestEvent`
- `codex-rs/cloud-state/src/request_store.rs`
  - `EventAppendParams`
  - `EventRecord`
  - `EventsPage`
- `codex-rs/app-server/src/bespoke_event_handling.rs`
  - 处理 TurnComplete / TurnAborted 等 core 事件，并转成 app-server 通知或 Request 终态

当前分支里的 Request Event 更偏“请求状态事件”，例如：

- `request/queued`
- `request/running`
- `request/completed`
- `request/failed`
- `request/interrupted`

需要区分两类事件：

| 事件类型 | 当前主要来源 | 是否完整 |
| --- | --- | --- |
| Request 状态事件 | `CloudRequestService` | 基础完成 |
| Turn 执行细节事件 | core/app-server listener | 还没有全部投影到 `request/events/list` |

如果产品要求客户端断线后可以从 `request/events/list` 完整补读模型输出和工具过程，就需要进一步做完整 EventLogStore 或事件投影。

### 3.5 RolloutItem

RolloutItem 是 Codex 恢复 Thread 历史的关键单元。可以粗略理解为“会话历史里可重放的一行记录”。

源码相关位置：

- `codex-rs/thread-store/src/types.rs`
  - `StoredThreadHistory { items: Vec<RolloutItem> }`
- `codex-rs/cloud-state/src/mysql_thread_store.rs`
  - `cloud_thread_rollout_items`
  - `append_items`
  - `load_history`
  - `read_thread`

为什么 RolloutItem 重要：

```text
节点 A 执行第一轮
  -> assistant 回复、工具结果等被写成 RolloutItem
  -> RolloutItem 存到共享 MySQL

节点 B 执行第二轮
  -> 读取同 threadId 的 RolloutItem 列表
  -> 重放成 Thread history
  -> 模型请求里带上第一轮上下文
```

没有 RolloutItem 或等价历史，跨节点只能知道“有这个 threadId”，不能知道“这个 Thread 之前说过什么”。

### 3.6 ThreadStore

ThreadStore 是“Thread 历史持久化接口”。它是你提到“只替换存储层”的核心落点。

源码相关位置：

- `codex-rs/thread-store/src/store.rs`
  - `pub trait ThreadStore`
- `codex-rs/thread-store/src/types.rs`
  - 各类参数和返回值
- `codex-rs/cloud-state/src/mysql_thread_store.rs`
  - 本分支新增的 MySQL 实现
- `codex-rs/core/src/thread_manager.rs`
  - core 通过 ThreadStore 创建、读取、恢复 Thread

ThreadStore 的主要方法：

| 方法 | 作用 |
| --- | --- |
| `create_thread` | 创建一个新 Thread 的持久化记录 |
| `resume_thread` | 重新打开已有 Thread 的持久化写入 |
| `append_items` | 追加 canonical rollout items |
| `load_history` | 读取 history，用于恢复、fork、rollback |
| `read_thread` | 读取 Thread 摘要，可选读取完整 history |
| `list_threads` | 列出 Thread |
| `archive_thread` / `unarchive_thread` | 归档和恢复归档 |

当前 ThreadStore 的一个结构性限制是：`AppendThreadItemsParams` 只有 `thread_id` 和 `items`，没有 `lease_id`、`fencing_token`、`append_idempotency_key`。因此，仅实现一个 MySQL ThreadStore 并不能天然防止多节点同时写同一个 Thread。并发写保护需要额外协议或上层保证。

### 3.7 ThreadManager

ThreadManager 是 core 里的 Thread 生命周期管理器。它负责启动、恢复、查找和移除正在运行的 Thread。

源码相关位置：

- `codex-rs/core/src/thread_manager.rs`

关键点：

| 方法 | 作用 |
| --- | --- |
| `start_thread_with_options` | 创建并启动一个新 Thread |
| `resume_thread_with_history` | 基于已有历史恢复 Thread |
| `resume_thread_from_rollout` | 从本地 rollout path 恢复 Thread |
| `resume_thread_from_store` | 当前补充：从 ThreadStore 按 threadId 读取 history 并恢复 |
| `get_thread` | 从当前进程内 map 查找已加载 Thread |
| `remove_thread` | 从进程内 map 移除 Thread |

这里有一个关键事实：`ThreadManager` 里有进程内线程表。即使 ThreadStore 换成 MySQL，当前节点也不一定已经加载这个 Thread。

因此跨节点请求需要：

```text
收到 threadId
  -> get_thread(threadId)
  -> 如果当前进程已加载，直接用
  -> 如果未加载，从 ThreadStore 读取 history
  -> resume_thread_with_history
  -> 再 turn/start
```

这就是为什么“替换存储层”通常还需要改一点入口执行路径：入口必须知道在找不到进程内 Thread 时触发 lazy resume。

### 3.8 Session / CodexThread

Session 和 CodexThread 是 core 内部执行模型的核心，但无状态化文档通常不需要深入到每个细节。

可以这样理解：

| 对象 | 简化理解 |
| --- | --- |
| `CodexThread` | 一个正在运行或可接收操作的 Thread 实例 |
| `Session` | 模型调用、工具调用、事件循环等内部运行上下文 |
| `Op::UserInput` | 向某个 Thread 提交用户输入，触发模型处理 |
| session loop | 后台循环，处理输入、模型事件、工具事件和终态 |

源码相关位置：

- `codex-rs/core/src/session`
- `codex-rs/core/src/thread_manager.rs`
- `codex-rs/app-server/src/request_processors/turn_processor.rs`

无状态化不要求把运行中的 Session 跨节点迁移。需求文档也明确首版不恢复运行中的 Turn。需要恢复的是“已经持久化的 Thread 历史”，不是内存里的 Session 对象。

## 4. 当前请求链路怎么走

入口的新旧关系如下。结论是：新增最多的是服务化 wrapper 和 cloud-state 层；原有核心是 Thread、Turn、ThreadStore、ThreadManager 和 app-server `turn/start`。若架构选择“不新增入口层”，应重点保留原有入口并扩展 lazy resume 和共享 ThreadStore，而不是保留完整 `request/*` wrapper。

| 入口/API | 原来是否存在 | 本分支变化 | 是否必须新增 |
| --- | --- | --- | --- |
| `thread/start` | 已存在 | 保留 | 不一定需要新增入口时仍可使用。 |
| `thread/resume` | 已存在 | 保留 | 本地/已有恢复入口。 |
| `turn/start` | 已存在 | cloud request 路径新增 lazy resume 能力 | 即使不新增 Request 入口，也需要让某条执行路径具备这个能力。 |
| `turn/interrupt` | 已存在 | 保留 | 可作为取消 running Turn 的底层能力。 |
| `request/run` | 新增 | 创建 Request、拿 lease、当前补充后启动真实 Turn | 如果外部系统承担 Request 语义，可以不在 Codex 内新增。 |
| `request/read` | 新增 | 查询 Request 状态 | 若外部系统承担状态查询，Codex 内可不需要。 |
| `request/events/list` | 新增 | 读取 Request 状态事件 | 若外部系统承担事件游标，Codex 内可不需要。 |
| `request/cancel` | 新增 | 写 cancelled 终态，后续可路由 owner interrupt | 若外部系统承担取消编排，Codex 内可不需要。 |
| `thread/appendWithLease` | 新增 | wrapper 层带 lease/fencing 的 append | 如果 ThreadStore 原生支持 lease-aware append，可能不需要这个 wrapper API。 |

### 4.1 现有 `turn/start` 链路

简化流程：

```text
客户端调用 app-server: turn/start(threadId, input)
  -> MessageProcessor 分发到 TurnRequestProcessor.turn_start
  -> turn_start_inner
  -> ThreadManager.get_thread(threadId)
  -> 找到进程内 CodexThread
  -> 提交 Op::UserInput
  -> core session loop 调模型和工具
  -> app-server listener 收事件
  -> 返回 TurnStartResponse
```

这个链路的问题是：如果当前进程没有加载该 Thread，`get_thread` 会失败。跨节点时这是常态。

### 4.2 当前补充后的 cloud request 链路

当前工作区补了 `start_turn_for_cloud_request` 和 `resume_thread_from_store`。

简化流程：

```text
request/run(threadId, input, idempotencyKey)
  -> CloudWrapperRequestProcessor.request_run
  -> CloudRequestService.run
  -> CloudStateStore.create_request
  -> acquire_thread_writer_lease
  -> Request 进入 running
  -> TurnRequestProcessor.start_turn_for_cloud_request
  -> ensure_cloud_thread_loaded
      -> get_thread(threadId)
      -> 不存在时 resume_thread_from_store
      -> 从 MysqlCloudThreadStore.read_thread(include_history=true)
      -> resume_thread_with_history
  -> turn_start_inner
  -> 真实启动 Turn
  -> set_request_turn_id
```

该链路说明：即使最终不保留 `request/run` 这个入口名，`ensure_cloud_thread_loaded` 这类 lazy resume 能力仍然需要存在于某条执行路径上。

### 4.3 Turn 终态如何回写 Request

当前补充后，Turn 完成或中断时会尝试更新 Request 终态。

简化流程：

```text
core 产生 TurnComplete 或 TurnAborted
  -> app-server bespoke_event_handling
  -> emit turn/completed 通知
  -> persist_cloud_request_terminal(turnId, Completed/Interrupted)
  -> CloudRequestService.terminal_for_turn_id
  -> 通过 turnId 找到 Request
  -> mark_request_terminal_with_event
```

这一步解决的是 Request 和 Turn 生命周期脱节的问题。没有它，`request/run` 可能创建了 Request 并启动了 Turn，但 Request 不知道 Turn 最终是否完成。

## 5. 存储层替换到底包括什么

如果按你的理解走“入口层不新增，只替换存储层”，最小需要替换或补齐的存储相关内容如下。

### 5.1 Thread 历史存储

必须有共享 ThreadStore。

当前实现：

- `MysqlCloudThreadStore`
- 表 `cloud_threads`
- 表 `cloud_thread_rollout_items`

目的：

```text
节点 A 写入 Thread history
节点 B 按 threadId 读取 Thread history
节点 B 恢复上下文并继续 turn/start
```

### 5.2 Request 状态存储

如果 Request 语义由 Codex 承担，则需要 `CloudStateStore`。

当前实现：

- `cloud_requests`
- `cloud_request_events`
- `cloud_thread_writer_leases`
- `cloud_thread_items`
- `cloud_append_results`

如果 Request 语义由外部系统承担，则 Codex 侧可以不需要这部分，或只保留必要的 ThreadStore。

### 5.3 并发写保护

只把 Thread history 放 MySQL，不等于自动安全。多节点并发同写一个 Thread 时，需要串行化。

可选方案：

| 方案 | 说明 |
| --- | --- |
| 外部系统串行化 | 外部调度保证同一 threadId 同时只有一个 running 请求 |
| Codex CloudStateStore writer lease | Codex 内部通过 lease/fencing 控制写入 |
| ThreadStore 自身 lease-aware | 修改 ThreadStore append 接口，写入时校验 fencing token |

当前分支实现了 CloudStateStore writer lease，但 ThreadStore 的原生 `append_items` 接口还不是 lease-aware。

## 6. 只替换存储层的可行边界

### 6.1 可行的最小架构

如果目标是不新增 Codex Request 入口，可以采用：

```text
外部调度系统
  -> 负责 Request 状态、幂等、排队、取消、owner 超时
  -> 调用 Codex 现有 thread/start / turn/start

Codex
  -> ThreadStore 换成共享 MySQL
  -> turn/start 支持 threadId lazy resume
  -> Thread append 受外部串行化或内部 lease 保护
  -> cloud runtime read-only 限制
```

这种方案下，Codex 不需要公开 `request/run`，但仍要改 `turn/start` 或其内部执行路径。

### 6.2 不可只靠存储层解决的点

| 问题 | 为什么不是纯存储问题 |
| --- | --- |
| 当前节点没有加载 Thread | 入口执行路径需要在 `get_thread` 失败后触发 resume |
| Request 幂等 | ThreadStore 不知道 callerId/idempotencyKey |
| queued/running/completed 状态 | ThreadStore 只保存 Thread，不保存服务请求状态 |
| 取消运行中请求 | 需要知道 owner 节点和 running Turn |
| owner 超时 | 需要心跳、扫描和终态写入 |
| 事件游标 | 需要 Request/Event 层定义读取顺序 |
| 多节点并发写 | 需要调度锁或 fencing，不是普通 append 就能保证 |

因此，“入口层不新增”可以成立；“只改存储层，入口路径完全不动”通常不成立。

## 7. 当前分支新增/修改的源码索引

### 7.1 协议与类型

| 文件 | 说明 |
| --- | --- |
| `codex-rs/cloud-wrapper-protocol/src/request.rs` | Request 状态、run/read/cancel/terminal 参数和响应 |
| `codex-rs/cloud-wrapper-protocol/src/append.rs` | 带 lease 的 Thread item append 协议 |
| `codex-rs/cloud-wrapper-protocol/src/event.rs` | Request event 协议 |
| `codex-rs/cloud-wrapper-protocol/src/runtime.rs` | 只读 Runtime capability profile |
| `codex-rs/app-server-protocol/src/protocol/common.rs` | app-server RPC 方法注册和请求枚举 |
| `codex-rs/app-server-protocol/src/protocol/v2/cloud_wrapper.rs` | v2 cloud wrapper 协议接入 |

### 7.2 云端状态

| 文件 | 说明 |
| --- | --- |
| `codex-rs/cloud-state/src/request_store.rs` | CloudStateStore trait、RequestRecord、EventRecord、lease 数据结构 |
| `codex-rs/cloud-state/src/request_service.rs` | Request run/read/cancel/terminal/events 的业务编排 |
| `codex-rs/cloud-state/src/mysql_store.rs` | MySQL CloudStateStore 实现 |
| `codex-rs/cloud-state/src/mysql_schema.rs` | MySQL 表结构和迁移 |
| `codex-rs/cloud-state/src/mysql_rows.rs` | MySQL row 到 Rust struct 的转换 |
| `codex-rs/cloud-state/src/mysql_thread_store.rs` | 当前补充的 MySQL ThreadStore |

### 7.3 app-server 执行路径

| 文件 | 说明 |
| --- | --- |
| `codex-rs/app-server/src/message_processor.rs` | app-server 总分发；当前 `request/run` 在这里桥接真实 Turn |
| `codex-rs/app-server/src/request_processors/cloud_wrapper_processor.rs` | Cloud wrapper processor |
| `codex-rs/app-server/src/request_processors/turn_processor.rs` | Turn start；当前补充 cloud request lazy resume |
| `codex-rs/app-server/src/bespoke_event_handling.rs` | Turn 完成/中断后同步 Request 终态 |
| `codex-rs/app-server/src/request_processors/thread_lifecycle.rs` | listener task 生命周期，传递 cloud wrapper processor |
| `codex-rs/app-server/src/request_processors/thread_processor.rs` | thread/start、resume 等处理器，传递 listener context |

### 7.4 core 与运行时

| 文件 | 说明 |
| --- | --- |
| `codex-rs/core/src/thread_manager.rs` | Thread 启动、恢复、进程内 registry；当前补充 `resume_thread_from_store` |
| `codex-rs/core/src/config/mod.rs` | cloud runtime 配置解析 |
| `codex-rs/config/src/config_toml.rs` | config.toml 中 cloud_runtime 字段 |
| `codex-rs/core/src/tools/handlers/apply_patch.rs` | cloud read-only 下拒绝 apply_patch |
| `codex-rs/core/src/tools/spec_plan.rs` | cloud read-only 下隐藏写工具/sub-agent 工具 |
| `codex-rs/core/src/hook_runtime.rs` | cloud read-only 下跳过 hooks |
| `codex-rs/core/src/tools/handlers/mcp.rs` | cloud read-only 下约束 MCP 工具 |

### 7.5 app-server 只读 RPC

| 文件 | 说明 |
| --- | --- |
| `codex-rs/app-server/src/request_processors/fs_processor.rs` | cloud read-only 下拒绝写文件、建目录、删除、复制 |
| `codex-rs/app-server/src/request_processors/process_exec_processor.rs` | cloud read-only 下拒绝 process/spawn |
| `codex-rs/app-server/src/request_processors/command_exec_processor.rs` | cloud read-only 下强制远程 read-only exec-server |
| `codex-rs/app-server/src/command_exec.rs` | command exec 具体执行编排 |

### 7.6 exec-server 只读环境

| 文件 | 说明 |
| --- | --- |
| `codex-rs/exec-server/src/read_only_scratch.rs` | read-only scratch 相关实现 |
| `codex-rs/exec-server/src/local_process.rs` | 本地进程执行和只读环境相关调整 |
| `codex-rs/exec-server/src/environment.rs` | environment 属性 |
| `codex-rs/exec-server/src/protocol.rs` | exec-server 协议字段 |

## 8. 关键数据表概念

当前分支 MySQL 相关表大致分两组。

### 8.1 Request/lease/event 表

| 表 | 作用 |
| --- | --- |
| `cloud_requests` | 保存 requestId、callerId、threadId、idempotencyKey、turnId、status、latest cursor |
| `cloud_request_events` | 保存 Request 级事件 |
| `cloud_thread_writer_leases` | 保存同 Thread writer lease |
| `cloud_thread_items` | 保存 wrapper 层 append 的 thread item payload ref |
| `cloud_append_results` | 保存 append 幂等结果 |

### 8.2 Thread history 表

| 表 | 作用 |
| --- | --- |
| `cloud_threads` | 保存 Thread 创建参数、metadata patch、归档时间 |
| `cloud_thread_rollout_items` | 保存可重放的 RolloutItem 历史 |

两组表的区别：

```text
cloud_requests 系列表：服务请求状态
cloud_threads 系列表：对话历史状态
```

## 9. 两种架构路线对源码的影响

### 9.1 路线 A：不新增 Codex 入口，外部承担 Request

Codex 必须做：

- `ThreadStore` 替换为 MySQL 实现。
- `turn/start` 或内部执行路径支持 lazy resume。
- Thread append 写入受外部串行化或内部 fencing 保护。
- 只读 Runtime 限制继续保留。

Codex 可不做：

- `request/run`
- `request/read`
- `request/events/list`
- `request/cancel`
- RequestStatus 存储

但这些能力要由外部系统实现，否则需求里“单次服务请求”的语义没人承担。

### 9.2 路线 B：Codex 内部提供 Request wrapper

Codex 做：

- Request 协议。
- CloudStateStore。
- request/run/read/events/cancel。
- writer lease。
- terminal 状态同步。
- ThreadStore MySQL。
- lazy resume。

当前分支更接近路线 B。

### 9.3 两条路线的共同必要项

无论是否新增入口，以下项都绕不开：

```text
共享 Thread history
  + 按 threadId 恢复 Thread
  + 同 Thread 写入串行化
  + 只读 Runtime
```

## 10. 对当前实现的源码级判断

当前分支已经形成三条主线：

1. **服务化 Request 主线**
   - `cloud-wrapper-protocol`
   - `cloud-state`
   - `CloudWrapperRequestProcessor`
   - `request/run/read/cancel/events`

2. **无状态 Thread 恢复主线**
   - `MysqlCloudThreadStore`
   - `ThreadManager::resume_thread_from_store`
   - `TurnRequestProcessor::ensure_cloud_thread_loaded`

3. **严格只读 Runtime 主线**
   - app-server fs/process/command 限制
   - core apply_patch/hooks/sub-agent/MCP 限制
   - exec-server read-only 环境

如果你的架构判断是“入口层不新增”，可以保留第 2 和第 3 条主线，弱化或删除第 1 条主线。但第 2 条主线中的 lazy resume 仍然是必要的，因为它解决的是当前节点进程内没有 Thread 的问题。

## 11. 建议你后续评审时重点看什么

不懂 Rust 时，不需要逐行看语法。建议按下面问题看源码：

1. 收到 `threadId` 时，当前节点没有加载 Thread 怎么办？
   - 看 `TurnRequestProcessor::ensure_cloud_thread_loaded`
   - 看 `ThreadManager::resume_thread_from_store`

2. 节点 B 怎么拿到节点 A 的历史？
   - 看 `MysqlCloudThreadStore::read_thread`
   - 看 `MysqlCloudThreadStore::load_history`
   - 看 `cloud_thread_rollout_items`

3. 同一个 Thread 会不会被两个节点同时写？
   - 看 `CloudStateStore::acquire_thread_writer_lease`
   - 看 `append_thread_items_with_lease`
   - 同时注意 `ThreadStore::append_items` 目前没有 lease 参数

4. `request/run` 有没有真的启动模型执行？
   - 看 `MessageProcessor` 的 `ClientRequest::RequestRun` 分支
   - 看 `TurnRequestProcessor::start_turn_for_cloud_request`

5. Turn 完成后 Request 怎么变 completed？
   - 看 `bespoke_event_handling.rs`
   - 看 `terminal_for_turn_id`

6. 云端只读是否有绕过路径？
   - 看 fs/process/command/apply_patch/hooks/sub-agent/MCP 这些入口是否都检查 `cloud_runtime.read_only`

## 12. 最小心智模型

可以把当前无状态化理解成三句话：

```text
Thread 是上下文，必须共享存储。
Turn 是一次执行，运行中可以留在某个节点。
Request 是服务化外壳，可以在 Codex 内部做，也可以在外部系统做。
```

由此可以推出架构边界：

```text
只替换 ThreadStore 可以解决“历史在哪里”。
lazy resume 解决“当前节点没有 Thread 怎么办”。
writer lease 解决“多个节点谁能写”。
Request 层解决“调用方怎么查状态、幂等、取消和补读事件”。
只读 Runtime 解决“模型和工具不能写环境”。
```

如果选择不新增入口层，就需要明确把最后一项 Request 层职责放到外部系统；否则系统仍缺少完整的服务请求语义。
