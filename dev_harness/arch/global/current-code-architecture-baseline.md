# 当前代码架构拆解基线

生成日期：2026-06-04  
分析范围：仓库当前工作区代码，重点覆盖 `codex-rs` Rust 工作区、顶层 npm 包装入口、app-server/TUI/核心会话运行链路。  
分析限制：本文基于静态代码读取和 Cargo metadata。未执行运行时集成测试，未验证外部服务、发布流程、CI 规则和实际客户端兼容矩阵。

## 1. 总体结论

当前仓库是以 Rust Cargo workspace 为主体的 Codex 本地代理系统。顶层 `package.json` 主要承担仓库维护脚本和 npm 包装分发；核心功能集中在 `codex-rs`，其中 `codex-cli` 提供统一命令入口，`codex-tui`、`codex-exec`、`codex-app-server`、`codex-mcp-server` 等 crate 承载不同交互形态，`codex-core` 承载主要业务逻辑。

从架构形态看，系统采用多入口、共享核心、协议隔离、异步会话编排的结构：

- `codex-cli` 是命令分发层，负责解析 CLI 子命令并转发到 TUI、exec、app-server、MCP server、sandbox、plugin、login 等功能模块。
- `codex-core` 是线程、会话、任务、上下文、工具、MCP、插件、权限、沙箱、rollout 等业务编排中心。
- `codex-protocol` 是核心内部协议模型，定义 `Op`、`Event`、`EventMsg`、`Submission`、`SandboxPolicy`、配置类型、模型项和权限模型。
- `codex-app-server-protocol` 是 app-server JSON-RPC API 合约层，维护 v1/v2 API、TypeScript/schema 导出和实验 API 标记。
- `codex-app-server` 是面向桌面端、IDE 或远程控制客户端的 JSON-RPC 服务端，连接客户端请求与 `ThreadManager`/`CodexThread`。
- `codex-tui` 是终端 UI 层，当前既有本地/旧核心适配，也通过 app-server client 支持 in-process 或 remote app-server 会话。

主要架构风险集中在两点：

1. `codex-core` 依赖和职责较多，已经成为业务逻辑、集成适配、运行时编排和部分 API 桥接的集中点。继续向其中添加新概念会增加变更耦合。
2. `codex-tui` 与 `codex-app-server` 文件量和行数较大，部分高触达模块仍承担较多流程编排职责。若继续扩展 UI 或 app-server RPC，需优先保持边界稳定，避免把新业务规则放入展示层或 request processor。

## 2. 仓库与工作区结构

### 2.1 顶层结构

顶层仓库包含：

- `codex-rs/`：主要 Rust 实现，当前 Cargo metadata 显示 workspace 内有 122 个 package。
- `codex-cli/`：npm 包 `@openai/codex`，提供 `bin/codex.js` 包装入口。
- `sdk/`：Python/TypeScript SDK 和 runtime 相关内容。
- `scripts/`、`tools/`、`third_party/`、`patches/`：构建、维护、第三方依赖和补丁相关内容。
- `dev_harness/`：本地架构、设计、开发、测试和工程辅助文档区域。

顶层 `package.json` 当前不是业务运行主入口，主要提供 repo-wide format 和 hook schema 生成脚本。实际产品行为由 Rust workspace 负责。

### 2.2 Rust 工作区规模

静态统计显示主要目录规模如下：

| 区域 | Rust 文件数 | 行数 |
| --- | ---: | ---: |
| `codex-rs/core/src` | 338 | 157194 |
| `codex-rs/tui/src` | 327 | 202158 |
| `codex-rs/app-server/src` | 68 | 39949 |
| `codex-rs/app-server-protocol/src` | 45 | 24645 |
| `codex-rs/protocol/src` | 30 | 18534 |
| `codex-rs/cli/src` | 29 | 18845 |

该规模说明当前系统的复杂度主要集中在 TUI 与 core，app-server-protocol 和 protocol 是重要边界但体量可控。

## 3. 分层模型

当前代码可按职责划分为以下层：

| 层级 | 代表 crate/目录 | 主要职责 |
| --- | --- | --- |
| 分发入口层 | `codex-cli`、`codex-cli/` npm wrapper | 统一命令入口、子命令分发、安装/包装入口 |
| 交互前端层 | `codex-tui`、`codex-exec`、`codex-app-server`、`codex-mcp-server` | 终端 UI、非交互执行、JSON-RPC 服务、MCP server |
| API 合约层 | `codex-protocol`、`codex-app-server-protocol` | 内部事件/操作协议、app-server v1/v2 JSON-RPC 类型、schema/TS 导出 |
| 核心编排层 | `codex-core` | thread/session/task/tool/context/MCP/plugin/sandbox/rollout 编排 |
| 配置与状态层 | `codex-config`、`codex-state`、`codex-rollout`、`codex-thread-store` | 配置加载、状态数据库、会话持久化、线程存储 |
| 工具与执行层 | `codex-exec-server`、`codex-sandboxing`、`codex-tools`、`codex-mcp` | 命令执行、沙箱策略、工具注册与执行、MCP client 管理 |
| 扩展与集成层 | `codex-plugin`、`codex-core-plugins`、`codex-extension-api`、`ext/*` | 插件、扩展、内置能力扩展 |
| 支撑工具层 | `utils/*`、`model-provider*`、`login`、`analytics`、`otel` | 路径、缓存、模型目录、认证、遥测等横切能力 |

分层并非严格单向。特别是 `codex-core` 同时依赖 `codex-app-server-protocol`，说明核心层存在面向 app-server 类型的耦合；`codex-tui` 通过 `codex_app_server_client::legacy_core` 引入旧核心适配，说明 TUI 正处在从直接核心耦合向 app-server 客户端形态迁移的状态。

## 4. 主要组件拆解

### 4.1 `codex-cli`

`codex-cli` 的二进制目标是 `codex`。`src/main.rs` 中 `MultitoolCli` 和 `Subcommand` 定义了统一 CLI 面：

- 无子命令时转向交互式 TUI。
- `exec`/`review` 提供非交互工作流。
- `app-server`、`remote-control`、`exec-server` 提供服务端或守护进程相关能力。
- `mcp`、`mcp-server` 提供 MCP 管理与 MCP server 模式。
- `plugin`、`cloud`、`sandbox`、`doctor`、`login/logout`、`apply` 等能力由对应 crate 或本地模块实现。

该层主要承担参数解析和启动配置拼装。架构上不应承载业务规则；业务规则应下沉到 core、app-server processor 或对应领域 crate。

### 4.2 `codex-core`

`codex-core` 是当前业务核心。`src/lib.rs` 显示其公开 API 包括：

- `ThreadManager`、`NewThread`、`CodexThread`：线程生命周期入口。
- `ModelClient`、`ResponseStream`、`Prompt`：模型请求相关抽象。
- `McpManager`、`SandboxState`：MCP 连接和沙箱状态相关能力。
- rollout/thread store 相关函数与类型。
- config、context、exec、exec_env、sandboxing、skills、review_format 等公开模块。

内部结构可进一步拆为：

- 线程管理：`thread_manager.rs` 负责创建、恢复、fork、shutdown、thread store 选择和共享服务装配。
- 线程通道：`codex_thread.rs` 包装 `Codex`，对外提供 `submit`、`submit_with_trace`、goal runtime 操作、rollout flush 等能力。
- 会话运行：`session/` 管理 `Session`、`SessionState`、输入队列、turn context、turn 生命周期、MCP 刷新、多 agent 和 review。
- 任务执行：`tasks/` 抽象 `SessionTask`，区分 regular、compact、review、user shell 等任务。
- 工具编排：`tools/` 通过 registry、router、orchestrator、lifecycle、parallel、sandboxing 等模块管理本地工具、MCP 工具、动态工具和 hook。
- 上下文构造：`context/` 与 `context_manager/` 管理 model-visible fragment、历史归一化和增量上下文更新。
- 权限与安全：`config/permissions*`、`exec_policy*`、`sandboxing/`、`guardian/`、`network_policy_decision*` 共同参与命令执行约束。

核心运行命题：

1. 客户端输入被转换为 `codex_protocol::protocol::Op`。
2. `CodexThread::submit` 将 `Op` 送入底层 `Codex`。
3. `Session` 串行化 turn 任务，一个 session 同时最多运行一个 task。
4. `SessionTask` 使用 `TurnContext` 构造模型请求、处理流式事件、执行工具并产生 `EventMsg`。
5. 事件经 core protocol 返回给 TUI、exec 或 app-server 映射层。

### 4.3 `codex-protocol`

`codex-protocol` 是 core 与各交互层共享的内部协议模型。关键类型包括：

- `Submission`：提交单元。
- `Op`：从客户端或上层入口进入 session 的操作。
- `Event`、`EventMsg`：session 向上层发出的事件。
- `SandboxPolicy`：运行时沙箱策略。
- `SessionConfiguredEvent`：session 初始化结果。
- `config_types`、`models`、`permissions`、`dynamic_tools`、`mcp`、`user_input` 等模块：支撑模型、权限、MCP 和用户输入结构。

该 crate 应保持为内部协议和领域类型边界。若加入 app-server 专用字段，应优先评估是否属于 `codex-app-server-protocol`。

### 4.4 `codex-app-server-protocol`

`codex-app-server-protocol` 负责 JSON-RPC 协议和 TypeScript/schema 导出。结构上分为：

- `jsonrpc_lite.rs`：JSON-RPC request/response/notification 基础模型。
- `protocol/common.rs`：ClientRequest、ClientResponse、ServerRequest、ServerNotification 等宏生成的公共协议集合。
- `protocol/v1.rs`：旧接口。
- `protocol/v2/*`：按资源拆分的 v2 API，包括 account、apps、config、environment、fs、mcp、model、plugin、process、realtime、thread、turn、review 等。
- `experimental_api.rs`：实验 API 标记与过滤机制。
- `export.rs`、`schema_fixtures.rs`：TS/schema 生成。

当前 v2 API 采用资源/方法语义，`thread.rs` 和 `turn.rs` 是核心交互入口：

- `ThreadStartParams` 定义线程启动模型、cwd、workspace roots、approval、sandbox、permissions、dynamic tools、environment 等参数。
- `TurnStartParams` 定义单 turn 输入、追加上下文、环境覆盖、cwd、权限、模型、service tier、collaboration mode 等参数。

该协议层已经较明确地隔离了 app-server 客户端合约。后续 API 变更应优先在此层完成类型定义和 schema 导出，再由 app-server processor 映射到 core。

### 4.5 `codex-app-server`

`codex-app-server` 是 JSON-RPC 服务端。主要结构：

- `main.rs`：解析 `--listen`、`--session-source`、auth、strict config、remote control 等启动参数。
- `lib.rs`：启动 app-server runtime，初始化配置、认证、环境管理、线程管理、transport 和 shutdown。
- `transport.rs` 与 `codex-app-server-transport`：stdio、unix socket、websocket、remote control 等连接形态。
- `message_processor.rs`：JSON-RPC 消息处理中心，维护初始化门禁、实验 API 门禁、请求序列化队列和各 processor。
- `request_processors/*`：按资源拆分 RPC 处理，包括 thread、turn、config、mcp、plugin、fs、git、process、account 等。
- `bespoke_event_handling.rs`：将 core event 映射为 app-server 通知，并处理 turn complete/interrupted/error、token usage、diff、plan update 等特殊事件。
- `thread_state.rs`、`thread_status.rs`：维护 app-server 视角的线程状态、turn history 和客户端订阅。

典型链路：

1. 客户端通过 stdio/unix/ws 建立连接。
2. transport 产出 `TransportEvent`。
3. `MessageProcessor` 对 JSON-RPC request 做初始化、实验 API 和序列化控制。
4. 对应 `RequestProcessor` 将 API 参数转换为 core 配置或 `Op`。
5. `ThreadManager` 创建/恢复 `CodexThread`，或向现有线程提交操作。
6. core 事件经监听任务进入 `bespoke_event_handling`。
7. app-server 发送 v2 notification/response 给客户端。

该层的主要职责应是协议适配、连接管理和状态投影，不宜承载 core turn 规则。

### 4.6 `codex-tui`

`codex-tui` 是终端交互 UI。结构上包括：

- `lib.rs`：启动、配置解析、auth、app-server client 选择、状态数据库初始化、TUI runtime 入口。
- `app.rs` 与 `app/*`：应用级事件分发、线程路由、app-server 事件处理、后台请求、会话生命周期。
- `chatwidget.rs` 与 `chatwidget/*`：聊天交互、输入提交、工具生命周期、权限弹窗、settings、realtime、插件、skills、turn runtime 等。
- `bottom_pane/*`：输入区、命令弹窗、审批 overlay、footer、skills/plugin/settings 等底部 UI。
- `history_cell/*`、`exec_cell/*`、`render/*`、`markdown*`：历史渲染、执行单元渲染、markdown/文本布局。
- `onboarding/*`、`notifications/*`、`ide_context/*`、`pets/*` 等辅助 UI 与集成能力。

`codex-tui` 当前直接依赖大量支撑 crate，并通过 `codex-app-server-client` 支持 in-process/remote app-server。`lib.rs` 中存在 `legacy_core` 适配，表明该层仍保留部分旧核心直接调用路径。

架构上，TUI 应保持为状态展示、输入采集和客户端请求编排层。长期方向应减少 TUI 对 core 具体规则的直接理解，优先通过 app-server protocol 表达行为。

### 4.7 `codex-exec` 与 `codex-mcp-server`

`codex-exec` 提供非交互模式。它依赖 app-server client、core、protocol、config、login 等，负责把命令行输入转换为自动化任务，并输出 human/jsonl 等格式。

`codex-mcp-server` 将 Codex 暴露为 MCP server。它直接依赖 `codex-core` 和 `codex-protocol`，用于让外部 MCP client 将 Codex 当作工具调用。

两者都是替代交互入口，核心差异是输出协议和调用方不同。

### 4.8 `codex-exec-server` 与沙箱/执行

`codex-exec-server` 负责命令、PTY、文件系统、HTTP/WebSocket relay 等执行服务能力，并依赖 `codex-sandboxing`、`codex-file-system`、`codex-protocol`、`codex-app-server-protocol` 等。

core 的工具执行路径通过 `tools/handlers/unified_exec`、`unified_exec/`、`sandboxing/`、`exec_policy*` 等模块使用执行服务与沙箱策略。执行相关能力横跨 core、exec-server、sandboxing、protocol permissions，属于高风险变更区域。

## 5. 关键运行链路

### 5.1 TUI 本地交互链路

```text
codex CLI
  -> codex-tui::run_main
  -> TUI App / ChatWidget
  -> app-server client 或 legacy core adapter
  -> ThreadManager / CodexThread
  -> Session
  -> SessionTask
  -> ModelClient / tools / MCP / sandbox execution
  -> EventMsg
  -> TUI event mapping and rendering
```

该链路中 UI 状态和 core 状态并存。需要重点控制两类问题：

- UI 是否根据协议事件完整更新，而不是依赖隐含 core 内部状态。
- 线程/turn 生命周期是否在 app-server client 与 legacy path 之间保持一致。

### 5.2 App-server 客户端链路

```text
client JSON-RPC
  -> transport
  -> MessageProcessor
  -> resource RequestProcessor
  -> ThreadManager / CodexThread
  -> core Session
  -> EventMsg
  -> bespoke_event_handling
  -> ServerNotification / JSON-RPC response
```

该链路的主要边界是 `codex-app-server-protocol`。API 变更需要同时考虑：

- Rust 类型与 TS/schema 导出一致性。
- v2 API 是否保持 camelCase wire 格式。
- 实验字段是否通过 `ExperimentalApi` 和 common gating 正确控制。
- app-server notification 是否能被客户端按线程和 turn 维度稳定消费。

### 5.3 非交互 exec 链路

```text
codex exec
  -> codex-exec::run_main
  -> app-server client 或 core
  -> session/task/model/tool execution
  -> event processor
  -> human/jsonl/final output
```

该链路要求输出可自动化消费，不能依赖 TUI 展示状态。错误处理、最后消息落盘、approval 行为和 sandbox 行为需要与交互链路保持语义一致。

### 5.4 MCP server 链路

```text
external MCP client
  -> codex-mcp-server
  -> codex-core ThreadManager / CodexThread
  -> core Session
  -> response/tool result exposed through MCP
```

该链路对外表现为 MCP 工具。由于 MCP 也是 core 中的重要 tool 输入来源，需要区分 Codex 作为 MCP client 与 Codex 作为 MCP server 的方向，避免概念混用。

## 6. 状态模型

当前系统至少存在以下状态域：

| 状态域 | 主要位置 | 说明 |
| --- | --- | --- |
| 配置状态 | `codex-config`、`codex-core/src/config`、app-server `ConfigManager` | 用户配置、managed config、profile、权限、模型配置 |
| 线程状态 | `ThreadManager`、`CodexThread`、`codex-thread-store` | 活跃线程集合、线程元数据、恢复/fork/archive |
| 会话状态 | `core/src/session`、`core/src/state` | 当前 turn、输入队列、任务状态、goal runtime、tool state |
| 持久化状态 | `codex-rollout`、`codex-state` | rollout 文件、state DB、log DB |
| App-server 投影状态 | `app-server/src/thread_state.rs`、`thread_status.rs` | 面向客户端的线程/turn 状态、订阅状态 |
| UI 状态 | `tui/src/app*`、`chatwidget*`、`bottom_pane*` | 当前视图、输入、弹窗、历史渲染、审批交互 |
| 执行环境状态 | `codex-exec-server`、`codex-sandboxing`、`EnvironmentManager` | 命令/PTY/环境/session runtime paths |
| 扩展状态 | `codex-extension-api`、plugins、skills、MCP manager | 插件、技能、MCP server、扩展生命周期 |

架构上需要避免状态重复带来的不一致。尤其是 app-server thread projection 与 core session state、TUI local state 与 app-server event state 之间，需要通过明确事件和快照边界保持一致。

## 7. 协议边界

### 7.1 内部协议边界

`codex-protocol` 是 core 与多个入口共享的内部协议。它不等价于 app-server 对外 API。其类型可服务于 TUI、exec、MCP、core tests 和 app-server 映射。

内部协议变更的影响面包括：

- core task/event 生成。
- TUI 渲染和交互状态更新。
- exec event processor。
- app-server event mapping。
- tests 和 rollout 重建逻辑。

### 7.2 App-server API 边界

`codex-app-server-protocol` 是 JSON-RPC 客户端可见边界。v2 API 已按资源拆分。该层需要对字段命名、实验 API、schema 生成和 TypeScript 导出保持严格一致。

API 变更不应直接由 app-server processor 中的 ad hoc JSON 结构表达，应先成为协议层的显式类型。

### 7.3 配置边界

配置跨越 `codex-config` 与 `codex-core/src/config`。当前 core 的 `SessionConfiguration` 仍保留 `original_config_do_not_use`，代码注释明确存在后续移除意图。这说明配置对象在 session 运行时仍存在历史耦合。

配置相关变更应明确区分：

- 启动时加载配置。
- thread 初始化快照。
- turn 级覆盖。
- 持久化 metadata。
- app-server 请求参数。

## 8. 依赖关系观察

Cargo metadata 显示以下关键依赖事实：

- `codex-core` 依赖 `codex-app-server-protocol`、`codex-config`、`codex-protocol`、`codex-exec-server`、`codex-mcp`、`codex-plugin`、`codex-rollout`、`codex-thread-store`、`codex-tools` 等大量内部 crate。
- `codex-app-server` 依赖 `codex-core`、`codex-app-server-protocol`、`codex-app-server-transport`、`codex-exec-server`、插件、扩展、MCP、state/thread-store 等。
- `codex-tui` 不直接依赖 `codex-core`，但通过 `codex-app-server-client::legacy_core` 访问旧核心适配，同时依赖 protocol、config、exec-server、plugin、state 等大量 crate。
- `codex-cli` 依赖几乎所有高层入口 crate，符合命令分发器特征。
- `codex-protocol` 依赖较少，是相对基础的内部模型层。

架构判断：

- `codex-core -> codex-app-server-protocol` 是需要关注的反向耦合。理想情况下，core 应只依赖内部 protocol 和领域抽象；app-server-protocol 应在 app-server 层完成映射。但现状可能是为了共享 thread history builder 或 API 映射类型，需逐项评估是否可迁移。
- `codex-tui` 的依赖数量较高，但其不直接依赖 `codex-core` 是正向信号。后续应继续减少 legacy_core 适配范围。
- `codex-app-server` request processor 的职责较多，但目录已按资源拆分，具备继续模块化的基础。

## 9. 主要架构风险

### 9.1 Core 职责集中

`codex-core` 当前包含线程管理、会话状态、任务、工具、上下文、MCP、插件注入、权限、沙箱、rollout、goal、guardian 等能力。其依赖面和公开 API 都较宽。

风险：

- 新功能容易默认进入 core，导致 core 更难拆分。
- app-server、TUI、exec、MCP server 的行为差异可能都需要触碰 core。
- 测试覆盖需要跨多个入口验证，单元级验证不足以证明行为一致。

建议：

- 新概念优先判断是否可放入专门 crate，例如协议、配置、工具、扩展、线程存储、执行环境。
- core 内新增逻辑优先靠近 `session/`、`tasks/`、`tools/` 等已有边界，不扩展 central orchestration 文件。
- 对 app-server 专用需求优先在 app-server processor 或 protocol 层实现，不反向污染 core。

### 9.2 UI 与业务规则边界不稳定

`codex-tui` 文件规模大，`app`、`chatwidget`、`bottom_pane` 仍覆盖大量交互流程。TUI 同时处理显示、输入、状态同步、权限弹窗、工具生命周期和 settings。

风险：

- UI 状态可能复制 core/app-server 状态，导致状态不一致。
- 新交互容易把业务规则写入 UI 层。
- 快照测试可覆盖渲染，但不必然覆盖协议语义。

建议：

- 新 UI 行为优先以 app-server protocol event/request 为边界。
- TUI 内仅保留展示和输入编排逻辑，业务约束放在 app-server/core。
- UI 变更必须补充 snapshot 或协议事件驱动测试。

### 9.3 App-server 事件映射复杂度

`bespoke_event_handling.rs` 承担 core event 到 app-server notification 的映射，并处理多类特殊事件。

风险：

- core event 新增或语义变化时，app-server notification 可能遗漏映射。
- 特殊处理逻辑过多会形成隐含协议。
- thread/turn 状态投影依赖事件顺序，异常路径需要额外覆盖。

建议：

- 新增 core event 时同步定义 app-server 映射策略。
- 对 turn completion、interruption、error、rollback、token usage 等关键事件保持集成测试。
- 将可类型化的映射逻辑沉淀到 protocol mapper 或独立模块，减少单文件增长。

### 9.4 协议双层模型带来的迁移成本

`codex-protocol` 和 `codex-app-server-protocol` 同时存在。前者偏内部，后者偏外部 JSON-RPC。两者之间需要映射。

风险：

- 类型名称相近但语义不同，容易产生错误复用。
- 外部 API 兼容约束可能通过类型依赖传导到 core。
- v1/v2 并存增加维护成本。

建议：

- 明确每个新字段属于内部事件还是外部 API。
- 外部 API 必须在 `app-server-protocol/src/protocol/v2` 中显式建模。
- 内部事件变更应通过 app-server mapper 显式转换，而不是直接复用外部 DTO。

### 9.5 权限、沙箱和执行链路跨度大

权限和执行链路跨越 `codex-protocol` permissions、`codex-config`、`codex-core` exec policy、`codex-sandboxing`、`codex-exec-server`、TUI/app-server approval 表达。

风险：

- 同一权限概念在配置、API、内部 session、执行器中存在多种表示。
- turn 级覆盖与 thread 级配置可能产生不一致。
- 平台差异会扩大测试矩阵。

建议：

- 变更权限语义时先定义状态转换矩阵：配置输入、thread snapshot、turn override、API response、执行策略。
- 对危险执行路径保持集成测试优先。
- 避免新增 bool/Option 参数表达权限模式，优先使用枚举或明确结构体。

## 10. 演进建议

### 10.1 短期：建立边界变更规则

建议在后续变更中采用以下规则：

- 新 app-server API：先改 `codex-app-server-protocol/src/protocol/v2/*`，再改 app-server processor，再改 core。
- 新 core agent 行为：优先加 core integration test，不以 TUI snapshot 代替行为测试。
- 新 UI 呈现：以协议事件为输入，补充 snapshot。
- 新工具能力：优先在 `codex-tools` 或 `core/src/tools` 既有注册/handler 结构中接入，避免绕过 tool lifecycle/hook/telemetry。
- 新持久化字段：同时评估 rollout、thread-store、state DB、resume/fork/backward compatibility。

### 10.2 中期：降低 core 反向耦合

建议逐步审计 `codex-core` 对 `codex-app-server-protocol` 的依赖。可按以下顺序处理：

1. 列出 core 中使用 app-server-protocol 类型的具体位置。
2. 判断类型是否本质属于内部协议、app-server DTO 或通用构建器。
3. 对内部协议类类型迁移到 `codex-protocol` 或专门 crate。
4. 对 app-server DTO 保持在 app-server 层映射。
5. 每次迁移保持小范围提交，避免同时改变 wire API。

### 10.3 中期：收敛 TUI legacy core path

建议明确 TUI 的目标架构：

- TUI 作为 app-server client。
- app-server in-process 模式作为本地运行默认路径。
- TUI 不直接理解 core session 内部结构。

在该目标下，逐步减少 `legacy_core` 适配面。每次迁移一个交互类别，例如 thread start/resume、turn start/steer、approval、settings、MCP、skills。

### 10.4 长期：按领域拆分核心能力

若继续扩展功能，建议避免继续扩大 `codex-core`。可优先评估以下拆分方向：

- context 构建与预算管理：独立于 session 编排。
- tool runtime 与 lifecycle：从 core 中提取为更稳定的工具运行层。
- permission profile resolution：从 config/core/app-server 交叉逻辑中收敛。
- thread lifecycle/store：保持 thread-manager 轻量化，持久化和生命周期策略下沉到专门 crate。
- event mapping：内部 event 到 app-server/TUI/exec 的投影逻辑独立化。

拆分条件应以实际依赖和测试可控为准，不应为目录整洁进行无行为收益的迁移。

## 11. 后续审计清单

建议后续架构审计按以下主题推进：

| 优先级 | 主题 | 目标 |
| --- | --- | --- |
| P0 | `codex-core` 对 `codex-app-server-protocol` 的依赖审计 | 明确反向耦合来源，制定迁移策略 |
| P0 | app-server turn/thread event mapping 覆盖 | 确认 turn 状态投影在异常路径稳定 |
| P1 | TUI legacy core adapter 范围 | 明确剩余直接核心语义，规划迁移 |
| P1 | 权限/沙箱状态转换矩阵 | 降低配置、API、执行策略之间的不一致风险 |
| P1 | thread resume/fork/rollback 持久化路径 | 验证 rollout、state DB、thread-store 的边界 |
| P2 | plugin/skills/MCP tool 注入链路 | 明确动态工具、技能和插件的上下文注入预算 |
| P2 | exec-server 与 unified exec 责任边界 | 确认执行生命周期、PTY、sandbox 和 app-server process exec 的职责划分 |

## 12. 判断边界

本文不主张立即进行大规模重构。当前代码已存在清晰的 crate 级分层和资源级 processor 拆分，主要问题不是缺少模块，而是部分核心职责和协议映射职责过于集中。

合理的演进路径是：在新增功能时保持边界单向、减少 core 依赖面、加强协议映射测试，并以小规模迁移逐步降低 TUI 和 app-server 对 core 内部细节的依赖。
