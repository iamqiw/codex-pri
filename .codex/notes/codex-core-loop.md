# Codex 源码核心 Loop 完整阅读笔记

> 本文是针对 `codex-rs` 中 session / thread / turn / task / tool / context / guardian 等核心机制的源码阅读笔记。
>
> 由于仓库 `AGENTS.md` 明确要求不要把通用产品或用户文档放入 `docs/`，本文放在本地源码学习笔记目录：`.codex/notes/`。

## 目录

1. 总览：Session / Thread / Turn / Task
2. Session 创建流程
3. 核心实体定义与关系
4. Core Loop 与 Submission / Event
5. Turn 执行闭环
6. Tool 系统
7. Skills / Plugins / Hooks 扩展系统
8. Multi-agent 体系
9. Approval / Sandbox / Guardian 安全边界
10. Context / History / Compaction / Rollout
11. 结构化上下文组装
12. ModelClient / ResponsesRequest / Streaming
13. 错误恢复 / Interrupt / Retry / Cleanup
14. 推荐源码阅读顺序

---

## 1. 总览：Session / Thread / Turn / Task

Codex 的核心运行模型可以先抽象成四层：

```text
Session
  长生命周期 runtime：配置、服务、状态、事件通道、active turn

Thread
  对话/历史/持久化维度：一个 conversation 的身份与 rollout 轨迹

Turn
  一次用户输入或系统输入驱动的逻辑轮次

Task
  当前 session loop 里真正正在跑的异步工作单元
```

几个关键点：

- `session` 是运行时容器，不只是一个 ID。
- `thread` 更偏对话历史和持久化身份。
- `turn` 是一次输入驱动的 agent 执行边界。
- `task` 是异步执行抽象，可能是普通任务、compact 任务、review 任务。
- 一个 session 一个时刻最多有一个 active turn / running task。
- 创建 session 不等于调用模型；只有收到 `Op::UserInput` 后才会启动 turn。

---

## 2. Session 创建流程

一次 session 创建大致是：

```text
外部入口
  ↓
加载 Config / Auth / ModelProvider / 权限策略
  ↓
Codex::spawn
  ↓
创建 submission/event channel
  ↓
加载 AGENTS.md / instructions
  ↓
解析 model/provider
  ↓
构造 SessionConfiguration
  ↓
Session::new
  ↓
创建 SessionState + SessionServices + ModelClient + MCP + persistence + telemetry
  ↓
构造 Arc<Session>
  ↓
发送 SessionConfigured
  ↓
初始化/刷新 MCP connection manager
  ↓
记录初始历史 / schedule prewarm
  ↓
spawn submission_loop
  ↓
等待后续 Op::UserInput
```

### 2.1 创建前：Config 决定 session 的边界

`Config` 决定这个 session：

- 用什么模型和 provider；
- 当前 `cwd` 是哪里；
- workspace roots 是什么；
- approval policy 是什么；
- permission profile / sandbox 是什么；
- 哪些 tools / MCP / plugins / skills 可用；
- 是否记录 rollout；
- 是否有 developer instructions / AGENTS.md；
- feature flags 和环境选择是什么。

这些配置会被整理为 `SessionConfiguration`，再在每个 turn 里派生出 `TurnContext`。

### 2.2 Auth 与 ModelClient

session 创建时会准备：

```text
AuthManager / API key / login state
  ↓
ModelProviderInfo
  ↓
ModelClient
```

`ModelClient` 是 session-scoped；每个 turn 会通过它创建一个 turn-scoped 的 `ModelClientSession`。

注意：Codex 的 `session_id` 不等于 OpenAI Responses API 的 `response_id`。Codex 内部有自己的 session/thread/window 概念；provider 侧可能还有 `previous_response_id`、sticky routing token 等协议层状态。

### 2.3 SessionConfigured

session 创建完成后会发：

```text
EventMsg::SessionConfigured(SessionConfiguredEvent)
```

它告诉 UI/app-server：

- `session_id`
- `thread_id`
- model / provider
- approval policy
- permission profile
- cwd
- parent/fork/thread source
- initial messages
- network proxy
- rollout path

这标志着 session runtime 已经可接收后续 turn。

---

## 3. 核心实体定义与关系

### 3.1 `Codex`

位置概念：`codex-rs/core/src/session/mod.rs`

```rust
pub struct Codex {
    pub(crate) tx_sub: Sender<Submission>,
    pub(crate) rx_event: Receiver<Event>,
    pub(crate) agent_status: watch::Receiver<AgentStatus>,
    pub(crate) session: Arc<Session>,
    pub(crate) session_loop_termination: SessionLoopTermination,
}
```

`Codex` 是外部交互壳：

- `tx_sub`：外部 -> core loop 的 submission 通道；
- `rx_event`：core loop -> 外部的事件通道；
- `agent_status`：watch 当前 agent 状态；
- `session`：真正的 runtime；
- `session_loop_termination`：等待 submission loop 退出。

关键方法：

- `Codex::spawn(args)`：创建 session 的入口；
- `submit(op)` / `submit_with_id(submission)`：发送操作；
- `next_event()`：读取事件；
- `shutdown_and_wait()`：提交 shutdown 并等待退出。

### 3.2 `CodexSpawnArgs`

这是 session 创建参数的大集合，主要包括：

```rust
pub(crate) struct CodexSpawnArgs {
    pub(crate) config: Config,
    pub(crate) installation_id: String,
    pub(crate) auth_manager: Arc<AuthManager>,
    pub(crate) models_manager: SharedModelsManager,
    pub(crate) environment_manager: Arc<EnvironmentManager>,
    pub(crate) skills_manager: Arc<SkillsManager>,
    pub(crate) plugins_manager: Arc<PluginsManager>,
    pub(crate) mcp_manager: Arc<McpManager>,
    pub(crate) extensions: Arc<ExtensionRegistry<Config>>,
    pub(crate) conversation_history: InitialHistory,
    pub(crate) session_source: SessionSource,
    pub(crate) forked_from_thread_id: Option<ThreadId>,
    pub(crate) parent_thread_id: Option<ThreadId>,
    pub(crate) thread_source: Option<ThreadSource>,
    pub(crate) agent_control: AgentControl,
    pub(crate) dynamic_tools: Vec<DynamicToolSpec>,
    pub(crate) metrics_service_name: Option<String>,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) inherited_exec_policy: Option<Arc<ExecPolicyManager>>,
    pub(crate) parent_rollout_thread_trace: ThreadTraceContext,
    pub(crate) user_shell_override: Option<shell::Shell>,
    pub(crate) parent_trace: Option<W3cTraceContext>,
    pub(crate) environment_selections: ResolvedTurnEnvironments,
    pub(crate) analytics_events_client: Option<AnalyticsEventsClient>,
    pub(crate) thread_store: Arc<dyn ThreadStore>,
    pub(crate) attestation_provider: Option<Arc<dyn AttestationProvider>>,
    pub(crate) inherited_multi_agent_version: Option<MultiAgentVersion>,
}
```

可以分成：

- 配置类：`config`、`installation_id`、metrics；
- 认证模型类：`auth_manager`、`models_manager`、attestation；
- 工具扩展类：`skills_manager`、`plugins_manager`、`mcp_manager`、`dynamic_tools`；
- thread/history 类：`conversation_history`、fork/parent/thread source、`thread_store`；
- agent 控制类：`agent_control`、multi-agent version；
- 执行环境类：shell snapshot、exec policy、environment selections。

### 3.3 `Session`

位置概念：`codex-rs/core/src/session/session.rs`

```rust
pub(crate) struct Session {
    pub(crate) thread_id: ThreadId,
    pub(crate) installation_id: String,
    pub(super) tx_event: Sender<Event>,
    pub(super) agent_status: watch::Sender<AgentStatus>,
    pub(super) out_of_band_elicitation_paused: watch::Sender<bool>,
    pub(super) state: Mutex<SessionState>,
    pub(super) managed_network_proxy_refresh_lock: Semaphore,
    pub(super) features: ManagedFeatures,
    pub(super) multi_agent_version: OnceLock<MultiAgentVersion>,
    pub(super) pending_mcp_server_refresh_config: Mutex<Option<McpServerRefreshConfig>>,
    pub(crate) conversation: Arc<RealtimeConversationManager>,
    pub(crate) active_turn: Mutex<Option<ActiveTurn>>,
    pub(crate) input_queue: InputQueue,
    pub(crate) guardian_review_session: GuardianReviewSessionManager,
    pub(crate) services: SessionServices,
    pub(super) next_internal_sub_id: AtomicU64,
}
```

`Session` 是核心 runtime。它持有：

- 当前 `thread_id`；
- 对外 event sender；
- agent status sender；
- `SessionState` 可变状态；
- feature flags；
- realtime conversation manager；
- 当前 `active_turn`；
- `InputQueue`；
- guardian review session manager；
- `SessionServices` 依赖集合；
- 内部 submission id 计数器。

`thread_id` 的来源：

```text
InitialHistory::New / Cleared / Forked => 创建新 ThreadId
InitialHistory::Resumed(resumed)       => 使用 resumed.conversation_id
```

`Session::session_id()` 通常从：

```rust
self.services.agent_control.session_id()
```

取得。多 agent 下，同一棵 agent tree 的多个 thread 可能共享同一个 session id。

### 3.4 `SessionConfiguration`

`SessionConfiguration` 是 session 的有效配置快照，不是完整 `Config`。

它包括：

- 模型/provider/reasoning/service tier/personality；
- base/developer/user instructions；
- compact prompt；
- approval policy / approvals reviewer；
- permission profile state / sandbox 相关配置；
- cwd / workspace roots / codex home；
- session source / fork / parent / thread source；
- dynamic tools；
- inherited shell snapshot / shell override。

关键方法：

- `permission_profile()`：materialize 权限 profile；
- `sandbox_policy()`：兼容生成 legacy sandbox policy；
- `file_system_sandbox_policy()`；
- `network_sandbox_policy()`；
- `thread_config_snapshot()`；
- `apply(updates)`：应用 thread settings 更新，返回新的配置快照。

### 3.5 `SessionState`

```rust
pub(crate) struct SessionState {
    pub(crate) session_configuration: SessionConfiguration,
    pub(crate) history: ContextManager,
    pub(crate) latest_rate_limits: Option<RateLimitSnapshot>,
    pub(crate) server_reasoning_included: bool,
    pub(crate) mcp_dependency_prompted: HashSet<String>,
    pub(crate) additional_context: AdditionalContextStore,
    previous_turn_settings: Option<PreviousTurnSettings>,
    auto_compact_window: AutoCompactWindow,
    pub(crate) startup_prewarm: Option<SessionStartupPrewarmHandle>,
    pub(crate) active_connector_selection: HashSet<String>,
    pub(crate) pending_session_start_sources: VecDeque<SessionStartSource>,
    granted_permissions_by_environment_id: HashMap<String, AdditionalPermissionProfile>,
    next_turn_is_first: bool,
}
```

`SessionState` 是 session-scoped 可变数据状态：

- 当前配置；
- 模型上下文 history；
- token/rate limit；
- MCP dependency 提示状态；
- additional context；
- 上一轮设置；
- auto compact 状态；
- active connectors；
- permission grants；
- first-turn 标记。

### 3.6 `SessionServices`

`SessionServices` 是 session 的依赖注入容器。核心字段包括：

- `mcp_connection_manager`；
- `unified_exec_manager`；
- `hooks: ArcSwap<Hooks>`；
- `rollout_thread_trace`；
- `user_shell` / shell snapshot；
- `exec_policy`；
- `auth_manager` / `models_manager`；
- `session_telemetry`；
- `tool_approvals`；
- guardian rejection/circuit breaker；
- `skills_manager` / `plugins_manager` / `mcp_manager` / `extensions`；
- `agent_control`；
- `network_proxy` / `network_approval`；
- `state_db` / `live_thread` / `thread_store`；
- `model_client`；
- `environment_manager`。

一句话：

```text
SessionState    = session 的数据状态
SessionServices = session 依赖的服务对象
```

### 3.7 `ModelClient` 与 `ModelClientSession`

`ModelClient` 是 session-scoped：

```rust
pub struct ModelClient {
    state: Arc<ModelClientState>,
    prompt_cache_key_override: Option<String>,
}
```

`ModelClientState` 里有：

- `session_id`
- `thread_id`
- `window_generation`
- provider
- session source / parent thread
- verbosity/compression/timing/attestation 配置
- websocket disable flag
- cached websocket session

`ModelClientSession` 是 turn-scoped：

```rust
pub struct ModelClientSession {
    client: ModelClient,
    websocket_session: WebsocketSession,
    turn_state: Arc<OnceLock<String>>,
}
```

它保存 per-turn 的 websocket session 和 sticky routing token。不同 turn 不能复用同一个 turn state。

### 3.8 `ContextManager`

```rust
pub(crate) struct ContextManager {
    items: Vec<ResponseItem>,
    history_version: u64,
    token_info: Option<TokenUsageInfo>,
    reference_context_item: Option<TurnContextItem>,
}
```

`ContextManager` 是模型可见 history 的核心容器。它保存的是结构化 `ResponseItem`，不是字符串数组。

关键字段：

- `items`：历史 item，旧在前，新在后；
- `history_version`：history 被 replace/compact/rollback 时递增；
- `token_info`：token 使用信息；
- `reference_context_item`：上下文 diff / cache 稳定性基准。

### 3.9 `ActiveTurn` / `RunningTask` / `TurnState`

`ActiveTurn`：

```rust
pub(crate) struct ActiveTurn {
    pub(crate) task: Option<RunningTask>,
    pub(crate) turn_state: Arc<Mutex<TurnState>>,
}
```

`RunningTask`：

```rust
pub(crate) struct RunningTask {
    pub(crate) done: Arc<Notify>,
    pub(crate) kind: TaskKind,
    pub(crate) task: Arc<dyn AnySessionTask>,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) handle: AbortOnDropHandle<()>,
    pub(crate) turn_context: Arc<TurnContext>,
    pub(crate) turn_extension_data: Arc<ExtensionData>,
    pub(crate) _timer: Option<codex_otel::Timer>,
}
```

`TurnState` 维护一个 turn 内部的 pending 状态：

- pending approvals；
- pending request permissions；
- pending user input；
- pending elicitations；
- pending dynamic tools；
- pending input queue；
- mailbox delivery phase；
- granted permissions；
- tool call count；
- token usage at turn start。

### 3.10 `Submission` / `Op` / `Event` / `EventMsg`

`Submission`：

```rust
pub struct Submission {
    pub id: String,
    pub op: Op,
    pub client_user_message_id: Option<String>,
    pub trace: Option<W3cTraceContext>,
}
```

`Op` 是外部提交给 core loop 的操作集合，例如：

- `UserInput`
- `Interrupt`
- `ThreadSettings`
- `ExecApproval`
- `PatchApproval`
- `InterAgentCommunication`
- `ResolveElicitation`
- `UserInputAnswer`
- `RequestPermissionsResponse`
- `DynamicToolResponse`
- `RefreshMcpServers`
- `Shutdown`

`Event`：

```rust
pub struct Event {
    pub id: String,
    pub msg: EventMsg,
}
```

`EventMsg` 是 core 对外输出的事件枚举，例如：

- `SessionConfigured`
- `TurnStarted`
- `AgentMessage`
- `AgentReasoning`
- `TokenCount`
- `TurnComplete`
- `TurnAborted`
- `Error`
- `Warning`
- tool / exec / MCP / patch 相关事件。

---

## 4. Core Loop 与 Submission / Event

`Codex` 外部不会直接调用模型，而是提交 `Submission`：

```text
client / TUI / app-server
  ↓
Codex::submit(Op)
  ↓
tx_sub: Sender<Submission>
  ↓
submission_loop
  ↓
match op
  ↓
启动/中断/响应/关闭 task
```

core loop 的职责：

- 接收 `Op`；
- 判断当前是否有 active task；
- 启动 regular/review/compact task；
- 把 approval/user input/elicitation response 送到对应 pending waiter；
- 处理中断和 shutdown；
- 发出 `EventMsg`；
- 保持 session 状态一致。

它是调度器，不是 agent 能力真正发生的地方。真正的 agent 能力发生在 turn execution 中。

---

## 5. Turn 执行闭环

一次普通 turn 大致是：

```text
Op::UserInput
  ↓
submission_loop
  ↓
start regular task
  ↓
create TurnContext
  ↓
build model request
  ↓
stream model response
  ↓
处理 assistant message / reasoning / tool call
  ↓
if tool call:
    execute tool
    append tool output
    continue model request
else:
    final answer
    turn complete
```

Turn execution 包含：

- TurnContext 构造；
- prompt/context 组装；
- model client streaming；
- response item 处理；
- tool dispatch；
- approval / guardian / sandbox；
- history record；
- token accounting；
- turn cleanup。

### 5.1 `TurnContext`

`TurnContext` 是单轮配置快照。它把 session 里的配置固化下来：

- model/provider；
- cwd；
- permission profile；
- approval policy；
- instructions；
- tools/dynamic tools；
- environment selections；
- network/sandbox；
- final output schema；
- turn skills；
- feature flags。

这样 turn 执行过程中不用频繁锁 `SessionState`。

### 5.2 为什么一个 turn 可能有多次模型请求

因为工具调用是中间步骤：

```text
request #1: 模型决定调用工具
  ↓
execute tool
  ↓
request #2: 模型读取 tool output 后继续
  ↓
可能再次 tool call
  ↓
request #N: 最终回答
```

所以 Codex 的 agent loop 不是单次 LLM call，而是模型和工具交替推进。

---

## 6. Tool 系统

Tool 系统是 Codex 从“聊天模型”变成“coding agent”的关键。

一次 tool call 流程：

```text
模型产生 tool call
  ↓
Codex 解析 ResponseItem / call id / arguments
  ↓
进入 tool registry / handler
  ↓
检查权限和上下文
  ↓
可能触发 approval / guardian
  ↓
在 sandbox / exec policy 下执行
  ↓
截断 / 结构化输出
  ↓
生成 tool output ResponseItem
  ↓
写回 ContextManager
  ↓
继续请求模型
```

### 6.1 Tool registry

Tool registry 告诉模型：

- 当前 turn 有哪些工具；
- 工具名是什么；
- 参数 schema 是什么；
- 描述和约束是什么。

工具可用性受以下因素影响：

- config；
- feature flags；
- approval policy；
- permission profile；
- MCP server 状态；
- agent mode；
- task kind；
- dynamic tools；
- environment。

### 6.2 内置工具与 MCP tools

内置工具包括：

- shell / exec；
- apply_patch；
- update_plan；
- request_user_input；
- request_permissions；
- spawn_agent；
- network approval。

MCP tools 流程：

```text
Session 初始化 MCP connection manager
  ↓
连接 MCP servers
  ↓
拉取 tool list
  ↓
转换为模型可见 schema
  ↓
模型调用 MCP tool
  ↓
Codex 转发给 MCP server
  ↓
MCP 返回结果
  ↓
Codex 回写 tool output
```

---

## 7. Skills / Plugins / Hooks 扩展系统

可以这样区分：

```text
plugin = 能力包 / 扩展容器
skill  = 任务流程 / prompt-level playbook
hook   = runtime 生命周期回调
tool   = 模型可调用的执行能力
```

### 7.1 Plugin

Plugin 是扩展分发单元，可以包含：

- skills；
- MCP servers / tools；
- apps / connectors；
- commands；
- hooks；
- config/defaults；
- metadata。

它的重点是发现、加载、注册能力。

### 7.2 Skill

Skill 不是执行引擎，而是能力注入机制。

```text
当前任务
  ↓
选择适用 skill
  ↓
读取 SKILL.md
  ↓
生成 TurnSkillsContext
  ↓
注入本轮 prompt
```

Skill 决定“怎么做”：

- 如何修 CI；
- 如何 review PR；
- 如何写测试；
- 如何发布 PR；
- 如何组合使用工具。

Tool 决定“能做什么”，Skill 决定“怎么做”。

### 7.3 Hook

Hook 是 runtime 主动触发的生命周期回调，不是模型主动调用的工具。

典型 hook 语义：

- session start；
- turn start；
- tool call begin/end；
- approval requested；
- turn complete；
- context compacted；
- shutdown。

`SessionServices` 中有：

```rust
pub(crate) hooks: ArcSwap<Hooks>
```

使用 `ArcSwap` 表示 hooks 可能随配置/plugin 刷新而动态替换。

---

## 8. Multi-agent 体系

Codex 的 agent 本质是“带 metadata 的 CodexThread”。

```text
agent_id == thread_id
```

每个 spawned agent 都有独立的：

- `CodexThread`
- `Session`
- `SessionState`
- `SessionServices`
- `ContextManager`
- `ModelClient`
- event stream
- submission loop

但同一棵 agent tree 共享：

- `AgentControl`
- `AgentRegistry`
- `session_id`
- 部分 inherited runtime：shell snapshot / exec policy

### 8.1 `AgentControl`

```rust
#[derive(Clone, Default)]
pub(crate) struct AgentControl {
    session_id: SessionId,
    manager: Weak<ThreadManagerState>,
    state: Arc<AgentRegistry>,
}
```

- `session_id`：同一棵 agent tree 共享；
- `manager`：弱引用回 ThreadManagerState，避免引用环；
- `state`：共享的 AgentRegistry。

### 8.2 `AgentRegistry`

`AgentRegistry` 维护：

- agent path -> metadata；
- nickname 去重；
- total count；
- spawn slot；
- root / spawned thread 注册；
- agent 释放。

`AgentMetadata`：

```rust
pub(crate) struct AgentMetadata {
    pub(crate) agent_id: Option<ThreadId>,
    pub(crate) agent_path: Option<AgentPath>,
    pub(crate) agent_nickname: Option<String>,
    pub(crate) agent_role: Option<String>,
    pub(crate) last_task_message: Option<String>,
}
```

### 8.3 `AgentPath`

`AgentPath` 是模型友好的逻辑路径：

```text
/root
/root/worker
/root/researcher
/root/researcher/worker
```

V2 多 agent 通信用 `AgentPath`，而不是让模型记 UUID thread id。

### 8.4 Spawn 流程

```text
spawn_agent tool call
  ↓
解析 role / path / prompt
  ↓
AgentControl::spawn_agent_with_metadata
  ↓
计算 multi_agent_version
  ↓
reserve max_threads slot
  ↓
继承 parent shell snapshot / exec policy
  ↓
prepare_thread_spawn
  ↓
ThreadManagerState::spawn_new_thread_with_source(config, self.clone(), ...)
  ↓
创建新的 CodexThread / Session
  ↓
注册 AgentMetadata
  ↓
持久化 parent -> child edge
  ↓
send_input(child_thread_id, initial_operation)
```

---

## 9. Approval / Sandbox / Guardian 安全边界

工具执行不是无条件的。安全链路大致是：

```text
模型请求动作
  ↓
识别 action 类型
  ↓
检查 PermissionProfile
  ↓
检查 ApprovalPolicy
  ↓
检查已有 grant / tool approval
  ↓
是否需要 reviewer？
    ├─ user approval
    └─ guardian auto-review
  ↓
得到 ReviewDecision
  ↓
如果 Approved：在 sandbox / exec policy 下执行
  ↓
生成 tool output
```

### 9.1 Approval 与 Sandbox 的区别

```text
approval = 是否允许这次动作
sandbox  = 即使允许，也只能在什么边界内执行
```

用户或 guardian 批准动作，不代表命令可以任意读写系统。最终仍受 filesystem/network/exec sandbox 约束。

### 9.2 Guardian

Guardian 是自动审批审查器。它判断：

> 某个需要 approval 的动作，能不能由系统自动批准，而不是展示给用户。

触发条件概念：

```text
approval_policy 是 OnRequest 或 Granular
并且 approvals_reviewer == AutoReview
```

Guardian 审查的请求类型包括：

- shell；
- exec command；
- execve；
- apply_patch；
- network access；
- MCP tool call；
- request permissions。

输出结构：

```rust
GuardianAssessment {
    risk_level,
    user_authorization,
    outcome: Allow | Deny,
    rationale,
}
```

### 9.3 Guardian 是特殊 sub-agent

Guardian session source 类似：

```rust
SessionSource::SubAgent(SubAgentSource::Other("guardian"))
```

但它不是普通 `ThreadSpawn` agent，不参与 agent tree 协作。

Guardian 自身受限：

- read-only sandbox；
- `approval_policy = never`；
- 禁用非必要 agent features；
- 不应 mutate state；
- 不应触发 further approvals。

### 9.4 Guardian fail closed

Guardian 的核心原则：

```text
明确 Allow => 执行
明确 Deny  => 拒绝
超时        => 拒绝/TimedOut
解析失败    => 拒绝
review session 失败 => 拒绝
```

### 9.5 低风险判断不是纯白名单，也不是纯靠大模型

实际是分层判断：

```text
确定性 permission / approval / sandbox 规则
  ↓
Guardian LLM 语义风险审查
  ↓
Sandbox / exec policy 强制边界
```

低风险综合判断：

- 是否和用户目标直接相关；
- 用户是否明确授权；
- 命令范围是否清晰；
- 是否只在 workspace 内；
- 是否写文件；
- 是否联网；
- 是否读取 secrets / home / system sensitive files；
- 是否破坏性操作；
- 是否可预测；
- 是否绕过权限或隐藏行为。

---

## 10. Context / History / Compaction / Rollout

Codex 需要维护两类相关但不同的数据：

```text
ContextManager = 模型可见上下文
Rollout        = 会话日志 / resume / replay / fork / UI 展示
```

### 10.1 History 写入

一次 turn 中，模型输出和工具结果会写入 history：

```text
model response item
  ↓
tool call
  ↓
tool output
  ↓
assistant final message
  ↓
SessionState.record_items(...)
  ↓
ContextManager.record_items(...)
```

### 10.2 Compaction

模型上下文窗口有限，history 变长后需要 compact：

```text
旧结构化 history
  ↓
compact task
  ↓
compact prompt
  ↓
模型生成 summary
  ↓
构造 compacted context item
  ↓
replace_history
  ↓
history_version += 1
window_generation += 1
```

Compact 不是简单丢弃旧消息，而是将旧上下文总结成更短但保留关键语义的 item。

### 10.3 Resume 与 Fork

Resume：

```text
已有 rollout
  ↓
ResumedHistory
  ↓
沿用 conversation_id 作为 thread_id
  ↓
重建 initial history
  ↓
继续同一个 thread
```

Fork：

```text
已有 rollout items
  ↓
InitialHistory::Forked(...)
  ↓
创建新 thread_id
  ↓
继承一段历史
  ↓
从某个点分叉继续
```

---

## 11. 结构化上下文组装

Codex 上下文组装不是拼一个大 prompt，而是维护多类结构化输入。

最终模型请求大致包括：

```text
Instructions
  base instructions
  developer instructions
  AGENTS.md
  selected skills
  environment rules

Input items
  user message
  assistant message
  tool call
  tool output
  additional context
  compact summary

Tool definitions
  builtin tools schema
  MCP tools schema
  dynamic tools schema
```

### 11.1 `ResponseItem` 是核心单位

历史不是 `Vec<String>`，而是 `Vec<ResponseItem>`。它保留：

- user message；
- assistant message；
- function/custom/local shell tool call；
- tool output；
- reasoning item；
- compacted context。

这样可以精确配对 tool call 和 tool output。

### 11.2 `Op::UserInput` 是结构化输入

`Op::UserInput` 包含：

- `items: Vec<UserInput>`；
- `environments`；
- `final_output_json_schema`；
- `responsesapi_client_metadata`；
- `additional_context`；
- `thread_settings`。

其中 `thread_settings` 会先应用到 session/turn 配置，而不是简单拼进 prompt。

### 11.3 Additional Context

`additional_context` 进入 `AdditionalContextStore`，然后被渲染成有边界的上下文 fragment。

目标：

- 支持 IDE/app-server 注入 selected text / open file / external result；
- 支持按 key 更新或替换；
- 支持 hard cap；
- 避免每轮无脑重复注入超大内容。

### 11.4 `reference_context_item` 与 cache 稳定性

`ContextManager.reference_context_item` 用来支持 context diff：

```text
哪些上下文上一轮已经注入？
哪些发生变化？
是否需要重新完整注入？
```

这能减少 token 浪费和 prompt cache miss。

### 11.5 `EventMsg` / `ResponseItem` / `RolloutItem`

三层必须分清：

```text
EventMsg     = 给 UI / client 看的运行事件
ResponseItem = 给模型看的下一轮上下文
RolloutItem  = 给持久化 / resume / replay / audit 的日志
```

不是所有事件都进模型，也不是所有模型 item 都等价于 UI event。

---

## 12. ModelClient / ResponsesRequest / Streaming

模型请求流程：

```text
TurnContext + ContextManager + tools
  ↓
build ResponsesRequest
  ↓
ModelClientSession sends request
  ↓
receive stream events
  ↓
parse / accumulate response state
  ↓
emit UI EventMsg
  ↓
collect ResponseItem
  ↓
detect tool calls
  ↓
update token/rate-limit info
  ↓
if tool calls:
    execute tools
    append tool outputs
    continue model request
else:
    final answer
    turn complete
```

### 12.1 Streaming event 不是 final message

模型 stream 可能包括：

- `response.created`；
- `response.output_item.added`；
- text delta；
- reasoning delta；
- function call arguments delta；
- output item done；
- response completed / failed / incomplete；
- usage / rate limit metadata。

Codex 需要把低层 event 转成：

- assistant 正在输出；
- reasoning summary/raw；
- tool call；
- complete response item；
- token/rate limit updates；
- error / warning。

### 12.2 Tool call 从 stream 中识别

模型可能先流出 tool call item，再流出 arguments delta。Codex 必须：

1. 识别 tool call 类型；
2. 收集完整 arguments；
3. 解析 JSON/schema；
4. 生成内部 tool call request；
5. 执行工具；
6. 生成带同一 call id 的 tool output；
7. 继续请求模型。

### 12.3 Provider continuation 与 Codex history

Provider 侧可能有：

- `previous_response_id`；
- `turn_state`；
- `window_id`；
- prompt cache key。

但 Codex 的权威上下文仍然是：

- `ContextManager.items`；
- `RolloutItem`；
- `thread_id`；
- `window_generation`。

---

## 13. 错误恢复 / Interrupt / Retry / Cleanup

真实运行中会遇到：

- 用户 interrupt；
- 模型 stream 中断；
- provider error；
- websocket failure；
- tool failure；
- approval timeout/deny；
- guardian deny；
- MCP error；
- malformed response。

### 13.1 Interrupt

```text
Op::Interrupt
  ↓
lock session.active_turn
  ↓
找到 RunningTask
  ↓
cancellation_token.cancel()
  ↓
abort / notify 当前 async task
  ↓
清理 pending approvals / user input / elicitations
  ↓
emit TurnAborted
  ↓
active_turn.task = None
```

Interrupt 不销毁 session，只取消当前 running task。

### 13.2 CancellationToken 与 AbortOnDropHandle

`CancellationToken` 是协作式取消，让模型 stream、tool execution、MCP call、approval wait 等 await 点主动退出。

`AbortOnDropHandle` 是兜底，保证 task 被 drop 时后台 tokio task 不继续跑。

### 13.3 Pending waiters 清理

`TurnState` 中 pending maps 包括：

- approvals；
- request permissions；
- user input；
- elicitations；
- dynamic tools。

中断/abort 时必须清理，否则会泄漏、卡死，或让后续 response 找错 turn。

### 13.4 Tool failure 分两类

业务失败：

```text
cargo test exit code != 0
```

这是有效结果，应作为 tool output 回灌给模型。

系统失败：

```text
sandbox 启动失败
MCP server disconnected
approval denied
guardian denied
malformed arguments
```

按情况变成 tool output、Error 或 TurnAborted。

### 13.5 History 写入边界

原则：

- 完整完成的 `ResponseItem` 才记录；
- 未完成 / malformed item 不污染 history；
- tool call 没有完整 arguments 不执行；
- tool output 必须和 call id 配对；
- abort 后旧 task 不应继续写 history。

---

## 14. 推荐源码阅读顺序

建议按这个顺序读：

```text
1. codex-rs/core/src/session/mod.rs
   Codex::spawn / submission_loop / Op 分发

2. codex-rs/core/src/session/session.rs
   Session::new / Session runtime 构造

3. codex-rs/core/src/state/session.rs
   SessionState / history / additional context / compact 状态

4. codex-rs/core/src/state/service.rs
   SessionServices / model / MCP / hooks / auth / exec / agent_control

5. codex-rs/core/src/state/turn.rs
   ActiveTurn / RunningTask / TurnState / pending waiters

6. codex-rs/core/src/session/turn_context.rs
   TurnContext 构造和 turn 配置快照

7. codex-rs/core/src/client.rs
   ModelClient / ModelClientSession / streaming / websocket fallback

8. codex-rs/core/src/tools/
   tool registry / handlers / shell / patch / request_user_input / request_permissions

9. codex-rs/core/src/mcp* 与 codex-rs/codex-mcp/
   MCP tool loading / mutation / tool call 转发

10. codex-rs/core/src/guardian/
    guardian approval request / review session / fail-closed / circuit breaker

11. codex-rs/core/src/context_manager/
    ResponseItem history / for_prompt / replace / token info

12. codex-rs/core/src/session/compact*
    compact task / history rewrite / window_generation

13. codex-rs/core/src/agent/
    AgentControl / AgentRegistry / spawn / AgentPath / inter-agent communication

14. codex-rs/protocol/src/protocol.rs
    Op / Event / EventMsg / SessionConfiguredEvent / InitialHistory
```

---

## 总结

Codex 的核心架构可以概括为：

```text
Codex
  外部交互壳：submit Op，receive Event

Session
  长生命周期 runtime：状态、服务、active turn、工具、事件

SessionConfiguration
  session 的有效配置快照

SessionState
  session 的可变数据状态：history、token、rate limit、additional context、permission grants

SessionServices
  session 依赖服务：model client、MCP、auth、hooks、skills/plugins、thread store、exec policy

TurnContext
  单轮 turn 的配置快照

ActiveTurn / RunningTask / TurnState
  当前 turn 的运行状态、任务句柄、pending waiters

ModelClient / ModelClientSession
  session-scoped 与 turn-scoped 的模型调用层

Tool system
  暴露工具、解析调用、执行动作、回写输出

Guardian / approval / sandbox
  决定动作是否可批准，以及在哪些硬边界内执行

ContextManager / Rollout
  分别维护模型可见上下文与可恢复会话日志

AgentControl / AgentRegistry
  管理多 agent tree、spawn、通信、关闭和状态跟踪
```

一句话：

> Codex 的 core loop 是调度心脏；turn execution 是 agent 主业务闭环；tool system 是执行层；skills/plugins/hooks 是扩展层；approval/sandbox/guardian 是安全边界；context/history/rollout 是记忆与恢复层；ModelClient/streaming 是模型协议层；错误恢复和 task cleanup 保证整个 runtime 长时间稳定运行。
